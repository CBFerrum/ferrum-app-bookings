//! Bookings: scheduling on insert (the engine under row locks), the guard that keeps times and
//! statuses in the hands of the actions, and the actions themselves — confirm, reschedule, cancel,
//! complete, no-show, invoice and guest links — plus the waitlist that refills class seats.
//!
//! Every state change bumps `notice_version`; maintenance turns each version into exactly one
//! notification (see `notify`). Actions write through `update_tx` with engine rights, so the
//! change is audited and published like any edit while users cannot set these fields themselves.

use crate::calendar::{self, ServiceInfo, StaffInfo};
use crate::engine::{self, Rules, Span, Why};
use crate::settings::{self, Settings};
use crate::{Instant, doc_id, int, obj, time, token, ts};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use ferrum_domain::flow::create_for_user;
use ferrum_domain::flow::created;
use ferrum_domain::util::{b, company, d, elevated, id, money, s, sys};
use ferrum_kernel::{Action, Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// Statuses that hold time (and seats, and resources).
pub const ACTIVE: [&str; 2] = ["Pending", "Confirmed"];

/// Where a booking goes, as decided by the engine under the staff and resource locks.
#[derive(Debug, Clone)]
pub struct Plan {
    pub staff: StaffInfo,
    pub resource: Option<Uuid>,
    /// The class session key; `None` for one-to-one bookings (they use their own id).
    pub session: Option<String>,
    pub waitlisted: bool,
    pub start: Instant,
    pub end: Instant,
    pub block: Span,
}

pub struct Request<'a> {
    pub svc: &'a ServiceInfo,
    pub st: &'a Settings,
    pub start: Instant,
    pub seats: i64,
    /// One staff member, or `None` for anyone who provides the service.
    pub staff: Option<Uuid>,
    /// The booking being moved: left out of the calendar it is checked against.
    pub exclude: Option<Uuid>,
    pub rules: Rules,
    /// Join a full class's waitlist instead of failing.
    pub waitlist: bool,
}

/// Locks the candidate staff and resources, reads their calendar and picks: the fair candidate,
/// a waitlist place, or an error saying why the time cannot be booked.
pub async fn plan(tx: &mut Tx, tenant: Uuid, r: &Request<'_>) -> Result<Plan> {
    let candidates: Vec<Uuid> = match r.staff {
        Some(s) if r.svc.staff.contains(&s) => vec![s],
        Some(_) => return Err(Error::field("staff", format!("does not provide {}", r.svc.name))),
        None => r.svc.staff.clone(),
    };
    calendar::lock_staff(tx, tenant, &candidates).await?;
    calendar::lock_resources(tx, tenant, &r.svc.resources).await?;
    let staff = calendar::staff(tx, tenant, &candidates).await?;
    if r.staff.is_some() && !staff.first().is_some_and(|s| s.active) {
        return Err(Error::field("staff", "is not taking bookings"));
    }
    let around = Span::new(r.start, r.start + Duration::minutes(r.svc.duration));
    let mut world = calendar::world(tx, tenant, r.svc, &staff, r.st, around, r.exclude).await?;
    world.rules = r.rules;
    let (ok, no) = world.candidates_at(r.start, r.seats);
    let svc = &world.service;
    let end = r.start + svc.duration;
    let block = Span::new(r.start - svc.before, end + svc.after);
    let found = |sid: Uuid| staff.iter().find(|s| s.id == sid).cloned().ok_or_else(|| Error::internal("candidate staff member vanished"));
    if let Some(c) = engine::pick(&ok) {
        return Ok(Plan { staff: found(c.staff)?, resource: c.resource, session: c.session.clone(), waitlisted: false, start: r.start, end, block });
    }
    if r.waitlist
        && let Some((sid, resource, key)) = world.full_session(r.start, r.seats)
    {
        return Ok(Plan { staff: found(sid)?, resource, session: Some(key), waitlisted: true, start: r.start, end, block });
    }
    Err(unavailable(&staff, &no, r))
}

fn unavailable(staff: &[StaffInfo], no: &[(Uuid, Why)], r: &Request<'_>) -> Error {
    if no.is_empty() {
        return Error::invalid(format!("no active staff member provides {}", r.svc.name));
    }
    let mut reasons: Vec<Why> = no.iter().map(|x| x.1).collect();
    reasons.sort();
    reasons.dedup();
    let mut text = reasons.iter().map(|w| w.message()).collect::<Vec<_>>().join("; ");
    if reasons.contains(&Why::Full) && !r.waitlist {
        text.push_str("; join the waitlist to get a seat that frees up");
    }
    let message = match no {
        [(one, _)] => {
            let name = staff.iter().find(|s| s.id == *one).map(|s| s.name.as_str()).unwrap_or("The staff member");
            format!("{name} is not available at that time ({text})")
        }
        _ => format!("no staff member is available at that time ({text})"),
    };
    if reasons.iter().any(|w| matches!(w, Why::Busy | Why::Full | Why::NoResource)) {
        Error::Conflict(message)
    } else if reasons == [Why::TooManySeats] {
        Error::field("seats", message)
    } else {
        Error::field("start_time", message)
    }
}

/// The scheduling fields of a booking placed by `p`.
pub fn schedule_fields(p: &Plan, booking: Uuid) -> Doc {
    obj(json!({
        "staff": p.staff.id.to_string(),
        "staff_name": p.staff.name,
        "staff_email": p.staff.email,
        "resource": p.resource.map(|r| r.to_string()),
        "start_time": ts(p.start),
        "end_time": ts(p.end),
        "block_start": ts(p.block.start),
        "block_end": ts(p.block.end),
        "session_key": p.session.clone().unwrap_or_else(|| booking.to_string()),
    }))
}

fn seats_of(doc: &Doc) -> i64 {
    doc.get("seats").and_then(Value::as_i64).unwrap_or(1)
}

fn check_seats(svc: &ServiceInfo, seats: i64) -> Result<()> {
    if (1..=svc.capacity).contains(&seats) {
        Ok(())
    } else {
        Err(Error::field("seats", format!("{} takes 1 to {} seat(s) per booking", svc.name, svc.capacity)))
    }
}

async fn service_of(tx: &mut Tx, tenant: Uuid, doc: &Doc) -> Result<ServiceInfo> {
    let sid = id(doc, "service").ok_or_else(|| Error::field("service", "is required"))?;
    calendar::service(tx, tenant, sid).await?.ok_or_else(|| Error::field("service", "does not exist"))
}

/// A new booking: the engine places it (staff, resource, session) and the derived fields follow.
async fn schedule_new(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    let tenant = hc.ctx.tenant;
    let st = settings::load(hc.tx, tenant).await?;
    let public = s(doc, "source") == Some("Public");
    let svc = service_of(hc.tx, tenant, doc).await?;
    if !svc.active {
        return Err(Error::field("service", format!("{} is not taking bookings", svc.name)));
    }
    let start = time(doc, "start_time").ok_or_else(|| Error::field("start_time", "is required"))?;
    let seats = seats_of(doc);
    check_seats(&svc, seats)?;
    let now = Utc::now();
    let rules = if public { st.public_rules(now) } else { Rules::default() };
    let request = Request { svc: &svc, st: &st, start, seats, staff: id(doc, "staff"), exclude: None, rules, waitlist: b(doc, "join_waitlist") };
    let p = plan(hc.tx, tenant, &request).await?;
    let booking = doc_id(doc)?;
    if let Some(c) = id(doc, "customer")
        && let Ok(cust) = hc.kernel.get_tx(&sys(hc.ctx), hc.tx, "Customer", c, false).await
    {
        for (to, from) in [("guest_name", "customer_name"), ("guest_email", "email"), ("guest_phone", "phone")] {
            if s(doc, to).is_none()
                && let Some(v) = s(&cust, from)
            {
                doc.insert(to.into(), json!(v));
            }
        }
    }
    let status = if p.waitlisted {
        "Waitlisted"
    } else if public && st.approval_required {
        "Pending"
    } else {
        "Confirmed"
    };
    doc.extend(schedule_fields(&p, booking));
    doc.extend(obj(json!({
        "service_name": svc.name,
        "location_type": svc.location_type,
        "location": svc.location,
        "item": svc.item.map(|i| i.to_string()),
        "rate": money(svc.price),
        "amount": money(svc.price * rust_decimal::Decimal::from(seats)),
        "status": status,
        "scheduled_at": ts(now),
        "confirmed_at": (status == "Confirmed").then(|| ts(now)),
        "notice_version": 1,
        "notified_version": 0,
        "last_change": "Booked",
    })));
    if id(doc, "company").is_none()
        && let Some(c) = st.company
    {
        doc.insert("company".into(), json!(c.to_string()));
    }
    if s(doc, "guest_time_zone").is_none() {
        doc.insert("guest_time_zone".into(), json!(hc.ctx.schema.timezone));
    }
    Ok(())
}

/// Writes `patch` to the booking with engine rights; `notify` bumps the notice version.
async fn update(k: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc, mut patch: Doc, notify: bool) -> Result<Doc> {
    if notify {
        patch.insert("notice_version".into(), json!(int(booking, "notice_version") + 1));
    }
    k.update_tx(ctx, tx, "Booking", doc_id(booking)?, patch, None).await
}

fn status(doc: &Doc) -> &str {
    s(doc, "status").unwrap_or_default()
}

pub async fn confirm(k: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc) -> Result<Doc> {
    if status(booking) != "Pending" {
        return Err(Error::invalid(format!("only pending bookings can be confirmed; this one is {}", status(booking))));
    }
    update(k, ctx, tx, booking, obj(json!({"status": "Confirmed", "confirmed_at": ts(Utc::now()), "last_change": "Confirmed"})), true).await
}

/// What a reschedule asks for: a new start, maybe another staff member (`Some(None)` = anyone),
/// maybe another number of seats.
#[derive(Debug, Clone)]
pub struct Move {
    pub start: Instant,
    pub staff: Option<Option<Uuid>>,
    pub seats: Option<i64>,
}

pub fn parse_move(params: &Value) -> Result<Move> {
    let start = params
        .get("start_time")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::field("start_time", "is required"))
        .and_then(|v| ferrum_kernel::value::parse_datetime(v).map_err(|m| Error::field("start_time", m)))?;
    let staff = match params.get("staff") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("any") || v.trim().is_empty() => Some(None),
        Some(Value::String(v)) => Some(Some(v.trim().parse().map_err(|_| Error::field("staff", "is not a staff id (or \"any\")"))?)),
        Some(_) => return Err(Error::field("staff", "is not a staff id (or \"any\")")),
    };
    let seats = match params.get("seats") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().or_else(|| v.as_str().and_then(|x| x.trim().parse().ok())).ok_or_else(|| Error::field("seats", "must be a whole number"))?),
    };
    Ok(Move { start, staff, seats })
}

/// Moves a booking (time, staff, seats) after checking the new time like a new booking — guests
/// within the reschedule window and on the offered grid. Seats freed in a class go to its waitlist.
pub async fn reschedule(k: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc, m: &Move, public: bool) -> Result<Doc> {
    if !ACTIVE.contains(&status(booking)) {
        return Err(Error::invalid(format!("only pending or confirmed bookings can be rescheduled; this one is {}", status(booking))));
    }
    let tenant = ctx.tenant;
    let st = settings::load(tx, tenant).await?;
    let now = Utc::now();
    let current = time(booking, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
    if public && now > current - st.reschedule_window {
        return Err(Error::invalid(format!(
            "this booking can no longer be rescheduled online (up to {} hours before it starts); please contact us",
            st.reschedule_window.num_hours()
        )));
    }
    let svc = service_of(tx, tenant, booking).await?;
    let seats = m.seats.unwrap_or_else(|| seats_of(booking));
    check_seats(&svc, seats)?;
    let staff = match m.staff {
        None => id(booking, "staff"),
        Some(choice) => choice,
    };
    let bid = doc_id(booking)?;
    // Every staff row this move can touch (the new candidates and the session it leaves, whose
    // waitlist moves up) is locked in one ordered batch, keeping the global lock order.
    let mut touched = svc.staff.clone();
    touched.extend(id(booking, "staff"));
    touched.sort();
    touched.dedup();
    calendar::lock_staff(tx, tenant, &touched).await?;
    let rules = if public { st.public_rules(now) } else { Rules::default() };
    let p = plan(tx, tenant, &Request { svc: &svc, st: &st, start: m.start, seats, staff, exclude: Some(bid), rules, waitlist: false }).await?;
    let old_session = s(booking, "session_key").map(str::to_string);
    let mut patch = schedule_fields(&p, bid);
    patch.extend(obj(json!({
        "seats": seats,
        "amount": money(d(booking, "rate") * rust_decimal::Decimal::from(seats)),
        "scheduled_at": ts(now),
        "last_change": "Rescheduled",
    })));
    let out = update(k, ctx, tx, booking, patch, true).await?;
    if svc.group()
        && let Some(old) = old_session
    {
        promote(k, ctx, tx, &svc, &st, &old).await?;
    }
    Ok(out)
}

/// Cancels a booking (guests within the cancellation window); a freed class seat goes to the waitlist.
pub async fn cancel(k: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc, reason: Option<&str>, public: bool) -> Result<Doc> {
    let was = status(booking).to_string();
    if !matches!(was.as_str(), "Pending" | "Confirmed" | "Waitlisted") {
        return Err(Error::invalid(format!("this booking is already {}", if was.is_empty() { "closed" } else { was.as_str() })));
    }
    let st = settings::load(tx, ctx.tenant).await?;
    if public && was != "Waitlisted" {
        let start = time(booking, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
        if Utc::now() > start - st.cancel_window {
            return Err(Error::invalid(format!(
                "this booking can no longer be cancelled online (up to {} hours before it starts); please contact us",
                st.cancel_window.num_hours()
            )));
        }
    }
    let reason: Option<String> =
        reason.map(|r| r.chars().filter(|c| !c.is_control() || *c == '\n').take(1000).collect::<String>().trim().to_string()).filter(|r| !r.is_empty());
    let patch = obj(json!({"status": "Cancelled", "cancellation_reason": reason, "cancelled_at": ts(Utc::now()), "last_change": "Cancelled"}));
    let out = update(k, ctx, tx, booking, patch, true).await?;
    if ACTIVE.contains(&was.as_str())
        && let Some(key) = s(booking, "session_key")
    {
        let svc = service_of(tx, ctx.tenant, booking).await?;
        if svc.group() {
            promote(k, ctx, tx, &svc, &st, key).await?;
        }
    }
    Ok(out)
}

/// Completed or No Show, once the booking has started (no guest notice).
pub async fn finish(k: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc, to: &str) -> Result<Doc> {
    let allowed: &[&str] = if to == "Completed" { &["Confirmed"] } else { &ACTIVE };
    if !allowed.contains(&status(booking)) {
        return Err(Error::invalid(format!("a {} booking cannot be marked {to}", status(booking).to_lowercase())));
    }
    let now = Utc::now();
    if time(booking, "start_time").is_some_and(|t| t > now) {
        return Err(Error::invalid(format!("a booking can be marked {to} once it has started")));
    }
    update(k, ctx, tx, booking, obj(json!({"status": to, "completed_at": ts(now)})), false).await
}

/// Gives freed seats of a class session to its waitlist, first come first served: each waiting
/// booking whose seats fit (re-checked like a booking, under the locks) is confirmed — or Pending
/// when public bookings need approval. Returns how many moved up.
pub async fn promote(k: &Kernel, ctx: &Ctx, tx: &mut Tx, svc: &ServiceInfo, st: &Settings, session: &str) -> Result<usize> {
    let waiting: Vec<(Uuid, Option<Uuid>, Instant, Option<i64>, Option<String>, Option<i64>)> = sqlx::query_as(
        "select id, staff, start_time, seats, source, notice_version from e_booking where tenant_id = $1 and session_key = $2 and status = 'Waitlisted' \
         and start_time > now() order by created_at, id for update skip locked",
    )
    .bind(ctx.tenant)
    .bind(session)
    .fetch_all(&mut **tx)
    .await?;
    let mut promoted = 0;
    for (wid, staff, start, seats, source, version) in waiting {
        let request = Request { svc, st, start, seats: seats.unwrap_or(1), staff, exclude: Some(wid), rules: Rules::default(), waitlist: false };
        let p = match plan(tx, ctx.tenant, &request).await {
            Ok(p) => p,
            // Still no room for this one (or it no longer fits the service): try the next.
            Err(Error::Conflict(_) | Error::Validation(_) | Error::Invalid(_)) => continue,
            Err(e) => return Err(e),
        };
        let to = if source.as_deref() == Some("Public") && st.approval_required { "Pending" } else { "Confirmed" };
        let mut patch = schedule_fields(&p, wid);
        patch.extend(obj(json!({
            "status": to,
            "confirmed_at": (to == "Confirmed").then(|| ts(Utc::now())),
            "last_change": "Promoted",
            "notice_version": version.unwrap_or(0) + 1,
        })));
        k.update_tx(ctx, tx, "Booking", wid, patch, None).await?;
        promoted += 1;
    }
    Ok(promoted)
}

/// `make_sales_invoice`: a draft Sales Invoice for the booking at the price it was booked at
/// (rate × seats, no discounts), once. A guest without a customer becomes one (matched by email
/// first). A cancelled or deleted invoice frees the booking to be invoiced again.
async fn invoice(hc: &mut HookCtx<'_>, booking: &Doc) -> Result<Value> {
    hc.ctx.require("SalesInvoice", Action::Create)?;
    let tenant = hc.ctx.tenant;
    let bid = doc_id(booking)?;
    if let Some(existing) = id(booking, "sales_invoice") {
        let row: Option<(i16, Option<String>)> = sqlx::query_as("select docstatus, number from e_sales_invoice where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(existing)
            .fetch_optional(&mut **hc.tx)
            .await?;
        if let Some((docstatus, number)) = row
            && docstatus != 2
        {
            return Err(Error::invalid(format!("this booking is already invoiced on {}", number.unwrap_or_else(|| "a draft sales invoice".into()))));
        }
    }
    let rate = d(booking, "rate");
    let (Some(item), true) = (id(booking, "item"), rate > rust_decimal::Decimal::ZERO) else {
        return Err(Error::invalid("nothing to invoice: the service was free when this was booked"));
    };
    let st = settings::load(hc.tx, tenant).await?;
    let company_id =
        id(booking, "company").or(st.company).ok_or_else(|| Error::invalid("set the company in Booking Settings (or on the booking) to invoice bookings"))?;
    let co = company(hc.kernel, hc.ctx, hc.tx, company_id).await?;
    let (customer, new_customer) = match id(booking, "customer") {
        Some(c) => (c, false),
        None => (customer_for(hc, booking).await?, true),
    };
    let tz = calendar::zone(&hc.ctx.schema.timezone).unwrap_or(chrono_tz::Tz::UTC);
    let when = time(booking, "start_time").map(|t| crate::when(t, tz)).unwrap_or_default();
    let number = s(booking, "number").unwrap_or_default().to_string();
    let seats = seats_of(booking).max(1);
    let input = obj(json!({
        "company": company_id.to_string(),
        "customer": customer.to_string(),
        "currency": co.currency_id.to_string(),
        "items": [{
            "item": item.to_string(),
            "qty": seats.to_string(),
            "price_list_rate": money(rate),
            "discount_percent": "0",
            "description": format!("{} on {when} ({number})", s(booking, "service_name").unwrap_or("Booking")),
        }],
        "remarks": format!("Booking {number}"),
    }));
    let inv = create_for_user(hc, "SalesInvoice", input).await?;
    let mut patch = obj(json!({"sales_invoice": inv.get("id").cloned().unwrap_or(Value::Null)}));
    if new_customer {
        patch.insert("customer".into(), json!(customer.to_string()));
    }
    hc.kernel.update_tx(&elevated(hc.ctx), hc.tx, "Booking", bid, patch, None).await?;
    Ok(created("SalesInvoice", &inv))
}

/// The guest's customer: an enabled one with their email, or a new individual customer.
async fn customer_for(hc: &mut HookCtx<'_>, booking: &Doc) -> Result<Uuid> {
    let email = s(booking, "guest_email");
    if let Some(e) = email {
        let found: Option<Uuid> =
            sqlx::query_scalar("select id from e_customer where tenant_id = $1 and lower(email) = lower($2) and not disabled order by created_at, id limit 1")
                .bind(hc.ctx.tenant)
                .bind(e)
                .fetch_optional(&mut **hc.tx)
                .await?;
        if let Some(c) = found {
            return Ok(c);
        }
    }
    let name = s(booking, "guest_name").ok_or_else(|| Error::invalid("the booking has neither a customer nor a guest name"))?;
    let input = obj(json!({"customer_name": name, "customer_type": "Individual", "email": email, "phone": s(booking, "guest_phone")}));
    let c = create_for_user(hc, "Customer", input).await?;
    doc_id(&c)
}

/// `guest_link`: a fresh self-service link to hand to the guest (when no email goes out).
async fn guest_link(hc: &mut HookCtx<'_>, booking: &Doc) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let st = settings::load(hc.tx, tenant).await?;
    let root = settings::site_root(hc.tx, tenant).await?;
    let token = token::mint(hc.tx, tenant, doc_id(booking)?).await?;
    let end = time(booking, "end_time").ok_or_else(|| Error::internal("booking without an end"))?;
    Ok(json!({
        "token": token,
        "url": st.manage_link(&root, &token),
        "api": st.absolute(&format!("{root}/manage/{token}")),
        "expires_at": ts(end + Duration::days(token::LINK_DAYS)),
        "note": "Anyone with this link can see, reschedule or cancel the booking: send it to the guest only.",
    }))
}

pub struct BookingHook;

#[async_trait]
impl DocHook for BookingHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // Stored bookings change only through the actions below.
        if doc.contains_key("created_at") {
            return Ok(());
        }
        schedule_new(hc, doc).await
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(z) = s(doc, "guest_time_zone")
            && calendar::zone(z).is_none()
        {
            return Err(Error::field("guest_time_zone", "is not an IANA time zone (e.g. Europe/Paris)"));
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if let Some(old) = old
            && !hc.ctx.is_system
        {
            for f in ["service", "staff", "start_time", "seats"] {
                if old.get(f) != doc.get(f) {
                    return Err(Error::field(f, "changes only through the Reschedule action"));
                }
            }
        }
        Ok(())
    }

    async fn before_delete(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if ACTIVE.contains(&status(doc)) {
            return Err(Error::invalid("cancel an active booking instead of deleting it (the guest is told, the time freed)"));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let engine = elevated(hc.ctx);
        let out = match name {
            "confirm" => confirm(hc.kernel, &engine, hc.tx, doc).await?,
            "reschedule" => reschedule(hc.kernel, &engine, hc.tx, doc, &parse_move(params)?, false).await?,
            "cancel_booking" => cancel(hc.kernel, &engine, hc.tx, doc, params.get("reason").and_then(Value::as_str), false).await?,
            "complete" => finish(hc.kernel, &engine, hc.tx, doc, "Completed").await?,
            "mark_no_show" => finish(hc.kernel, &engine, hc.tx, doc, "No Show").await?,
            "make_sales_invoice" => return Ok(Some(invoice(hc, doc).await?)),
            "guest_link" => return Ok(Some(guest_link(hc, doc).await?)),
            _ => return Ok(None),
        };
        Ok(Some(Value::Object(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reschedule_parameters() {
        let m = parse_move(&json!({"start_time": "2026-11-02T10:00:00+05:30"})).unwrap();
        assert_eq!(ts(m.start), "2026-11-02T04:30:00.000Z");
        assert!(m.staff.is_none() && m.seats.is_none(), "staff and seats stay as they are");
        let any = parse_move(&json!({"start_time": "2026-11-02T10:00:00Z", "staff": "any", "seats": "2"})).unwrap();
        assert_eq!((any.staff, any.seats), (Some(None), Some(2)));
        let who = Uuid::now_v7();
        assert_eq!(parse_move(&json!({"start_time": "2026-11-02T10:00:00Z", "staff": who.to_string()})).unwrap().staff, Some(Some(who)));
        assert!(parse_move(&json!({})).is_err());
        assert!(parse_move(&json!({"start_time": "tomorrow"})).is_err());
        assert!(parse_move(&json!({"start_time": "2026-11-02T10:00:00Z", "staff": 7})).is_err());
        assert!(parse_move(&json!({"start_time": "2026-11-02T10:00:00Z", "seats": "two"})).is_err());
    }
}
