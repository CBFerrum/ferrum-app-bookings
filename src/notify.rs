//! Notifications, sent by maintenance — each exactly once.
//!
//! A booking's guest-visible changes bump its `notice_version`; maintenance sends one notice per
//! version (to the guest by email through the Mail app when it is installed, to the staff member
//! in-app) and reminders at the configured offsets. Every send first inserts a claim row
//! (`bookings_notice`, primary key booking + kind + key) in the same transaction that sends, so a
//! retry, a second replica or a crash in between can never send twice. Guests get a fresh
//! self-service link in every email.

use crate::booking;
use crate::calendar;
use crate::engine::Span;
use crate::ics::{self, Event};
use crate::settings::{self, Settings};
use crate::{Instant, int, obj, time, token, when};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use ferrum_domain::util::{id, s};
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx, with_retry};
use ferrum_meta::Schema;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// What a guest is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guest {
    Requested,
    Confirmed,
    Waitlisted,
    Rescheduled,
    Cancelled,
    Reminder,
    /// Its deposit was not paid in time: the time was released.
    Expired,
    /// Its deposit came after the time was released and taken: it goes back.
    Refunded,
}

/// What the staff member is told in-app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaffNote {
    New,
    AwaitsConfirmation,
    Rescheduled,
    Cancelled,
    SeatFilled,
    /// Their outside calendar became busy at a booking's time after it was booked (FEO-032).
    Clash,
    /// A booking's deposit was not paid in time: the time is free again.
    Expired,
    /// A confirmed booking, or class session, is coming up.
    Reminder,
}

/// The messages for a change notice, from the booking's current state and whether anything was
/// sent before (`first`): the first notice always describes the booking as it is now, so a booking
/// made and moved within a minute is announced once, at its new time.
pub fn change_messages(status: &str, last_change: &str, first: bool) -> (Option<Guest>, Option<StaffNote>) {
    match (status, first) {
        ("Expired", _) if last_change == "Refunded" => (Some(Guest::Refunded), None),
        ("Expired", true) => (Some(Guest::Expired), None),
        ("Expired", false) => (Some(Guest::Expired), Some(StaffNote::Expired)),
        ("Cancelled", true) => (None, None),
        ("Cancelled", false) => (Some(Guest::Cancelled), Some(StaffNote::Cancelled)),
        ("Waitlisted", true) => (Some(Guest::Waitlisted), None),
        ("Pending", true) => (Some(Guest::Requested), Some(StaffNote::AwaitsConfirmation)),
        ("Confirmed", true) => (Some(Guest::Confirmed), Some(StaffNote::New)),
        ("Pending" | "Confirmed", false) => match last_change {
            "Rescheduled" => (Some(Guest::Rescheduled), Some(StaffNote::Rescheduled)),
            "Restored" => (Some(Guest::Confirmed), Some(StaffNote::New)),
            "Promoted" if status == "Pending" => (Some(Guest::Requested), Some(StaffNote::AwaitsConfirmation)),
            "Promoted" => (Some(Guest::Confirmed), Some(StaffNote::SeatFilled)),
            "Confirmed" => (Some(Guest::Confirmed), None),
            _ => (None, None),
        },
        _ => (None, None),
    }
}

pub fn mail_installed(ctx: &Ctx) -> bool {
    ctx.schema.packages.iter().any(|p| p == "comms") && ctx.schema.entity("Communication").is_some()
}

/// Inserts the claim row; `false` when this notification was already claimed.
async fn claim(tx: &mut Tx, tenant: Uuid, booking: Uuid, kind: &str, key: &str, channel: &str) -> Result<bool> {
    Ok(sqlx::query("insert into bookings_notice (tenant_id, booking_id, kind, key, channel) values ($1, $2, $3, $4, $5) on conflict do nothing")
        .bind(tenant)
        .bind(booking)
        .bind(kind)
        .bind(key)
        .bind(channel)
        .execute(&mut **tx)
        .await?
        .rows_affected()
        == 1)
}

async fn set_channel(tx: &mut Tx, tenant: Uuid, booking: Uuid, kind: &str, key: &str, channels: &[&str]) -> Result<()> {
    let channel = if channels.is_empty() { "none".to_string() } else { channels.join("+") };
    sqlx::query("update bookings_notice set channel = $5 where tenant_id = $1 and booking_id = $2 and kind = $3 and key = $4")
        .bind(tenant)
        .bind(booking)
        .bind(kind)
        .bind(key)
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The guest's zone (or the workspace's).
pub fn guest_zone(b: &Doc, ctx: &Ctx) -> Tz {
    s(b, "guest_time_zone").and_then(calendar::zone).or_else(|| calendar::zone(&ctx.schema.timezone)).unwrap_or(Tz::UTC)
}

/// The booking as a calendar event (guest download, invitations, staff feed).
pub fn booking_event(b: &Doc, summary: String, status: &'static str) -> Option<Event> {
    let (start, end) = (time(b, "start_time")?, time(b, "end_time")?);
    let mut description = format!("Reference {}", s(b, "number").unwrap_or_default());
    if let Some(l) = s(b, "location").filter(|_| s(b, "location_type") != Some("In Person")) {
        description.push_str(&format!("\n{}: {l}", s(b, "location_type").unwrap_or("Location")));
    }
    Some(Event {
        uid: format!("{}@bookings.ferrum", s(b, "id")?),
        sequence: int(b, "notice_version"),
        stamp: Utc::now(),
        start,
        end,
        summary,
        description: Some(description),
        location: s(b, "location").map(str::to_string),
        status,
        organizer: s(b, "staff_email").map(|e| (s(b, "staff_name").unwrap_or(e).to_string(), e.to_string())),
        attendees: s(b, "guest_email").map(|e| (s(b, "guest_name").unwrap_or(e).to_string(), e.to_string())).into_iter().collect(),
    })
}

/// The template a guest's message is written from; a booking waiting for its deposit is asked for it.
pub(crate) fn template_for(kind: Guest, deposit_due: bool) -> &'static str {
    match kind {
        Guest::Requested if deposit_due => "bookings.guest_deposit_due",
        Guest::Requested => "bookings.guest_requested",
        Guest::Confirmed => "bookings.guest_confirmed",
        Guest::Waitlisted => "bookings.guest_waitlisted",
        Guest::Rescheduled => "bookings.guest_rescheduled",
        Guest::Cancelled => "bookings.guest_cancelled",
        Guest::Reminder => "bookings.guest_reminder",
        Guest::Expired => "bookings.guest_expired",
        Guest::Refunded => "bookings.guest_refunded",
    }
}

/// The template a staff notice is written from.
fn staff_template(note: StaffNote) -> &'static str {
    match note {
        StaffNote::New => "bookings.staff_new",
        StaffNote::AwaitsConfirmation => "bookings.staff_awaits_confirmation",
        StaffNote::Rescheduled => "bookings.staff_rescheduled",
        StaffNote::Cancelled => "bookings.staff_cancelled",
        StaffNote::SeatFilled => "bookings.staff_seat_filled",
        StaffNote::Clash => "bookings.staff_clash",
        StaffNote::Expired => "bookings.staff_expired",
        StaffNote::Reminder => "bookings.staff_reminder",
    }
}

/// The deposit as a guest's message says it: "300.00 INR".
fn deposit_text(deposit: &Value) -> String {
    let amount = deposit["amount"].as_str().and_then(|a| a.parse::<rust_decimal::Decimal>().ok()).map(|a| format!("{:.2}", a)).unwrap_or_default();
    format!("{amount} {}", deposit["currency"].as_str().unwrap_or_default()).trim().to_string()
}

/// Emails the guest with a fresh self-service link: through the Mail app when it is installed
/// (with a calendar invitation for confirmations, moves and cancellations), otherwise through the
/// server's own mailer (the invitation is a link in the email). `false` when there is no way to
/// send, or no usable address.
async fn email_guest(k: &Kernel, ctx: &Ctx, tx: &mut Tx, b: &Doc, kind: Guest, st: &Settings) -> Result<bool> {
    let Some(to) = s(b, "guest_email") else { return Ok(false) };
    let via_mail_app = mail_installed(ctx);
    if !via_mail_app && k.mailer.is_none() {
        return Ok(false);
    }
    // The links leave the workspace: absolute, through the operator's link host when there is one.
    let st = &st.clone().reached(k, ctx.tenant).await;
    let bid = id(b, "id").ok_or_else(|| Error::internal("booking without id"))?;
    let root = settings::site_root(tx, ctx.tenant).await?;
    let token = token::mint(tx, ctx.tenant, bid).await?;
    let manage = st.manage_link(&root, &token);
    let ics_url = st.absolute(&format!("{root}/manage/{token}/booking.ics"));
    let tz = guest_zone(b, ctx);
    let deposit = if ctx.schema.packages.iter().any(|p| p == crate::payments::PACKAGE) { crate::payments::guest_view(ctx, tx, b).await? } else { Value::Null };
    let payable = deposit["status"] == "Initiated";
    let template = template_for(kind, payable);
    let mut values = json!({
        "when": time(b, "start_time").map(|t| when(t, tz)).unwrap_or_default(),
        "until": time(b, "end_time").map(|t| t.with_timezone(&tz).format("%H:%M").to_string()).unwrap_or_default(),
        "manage_link": manage,
        "calendar_link": ics_url,
    });
    if matches!(template, "bookings.guest_deposit_due" | "bookings.guest_expired") {
        // The guest's own page is the way to pay: the gateway's address may not exist yet, and stops working.
        values["deposit"] = json!(deposit_text(&deposit));
        values["pay_by"] =
            json!(deposit["pay_by"].as_str().and_then(|t| ferrum_kernel::value::parse_datetime(t).ok()).map(|t| when(t, tz)).unwrap_or_default());
        values["pay_link"] = json!(if payable { manage.as_str() } else { "" });
    }
    let m = Message {
        template: template.into(),
        entity: Some("Booking".into()),
        record: Some(bid),
        to: vec![Recipient {
            email: Some(to.to_string()),
            language: s(b, "guest_language").map(str::to_string),
            time_zone: s(b, "guest_time_zone").map(str::to_string),
            ..Default::default()
        }],
        channels: vec!["email".into()],
        values: values.as_object().cloned().unwrap_or_default(),
        ..Default::default()
    };
    if !via_mail_app {
        return match messages::send(k, tx, ctx, &m).await {
            Ok(sent) => {
                if let Some(skipped) = sent.iter().find(|x| x.state == "skipped") {
                    tracing::warn!(booking = %bid, "guest email not sent: {:?}", skipped.reason);
                }
                Ok(sent.iter().any(|x| x.channel == "email" && x.state == "queued"))
            }
            Err(Error::Validation(e)) => {
                tracing::warn!(booking = %bid, "guest email not sent: {e:?}");
                Ok(false)
            }
            Err(e) => Err(e),
        };
    }
    let rendered = match messages::render_for(k, tx, ctx, &m, "email").await {
        Ok(r) => r,
        Err(Error::Forbidden(e)) => {
            tracing::warn!(booking = %bid, "guest email not sent: {e}");
            return Ok(false);
        }
        Err(Error::Validation(e)) => {
            tracing::warn!(booking = %bid, "guest email not sent: {e:?}");
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    let summary = format!("{} with {}", s(b, "service_name").unwrap_or("Booking"), s(b, "staff_name").unwrap_or("us"));
    let invite = match kind {
        Guest::Confirmed | Guest::Rescheduled => booking_event(b, summary, "CONFIRMED").map(|e| ics::calendar(Some("REQUEST"), None, &[e])),
        Guest::Cancelled | Guest::Expired => booking_event(b, summary, "CANCELLED").map(|e| ics::calendar(Some("CANCEL"), None, &[e])),
        _ => None,
    };
    let mail = obj(json!({
        "channel": "Email",
        "direction": "Sent",
        "recipients": to,
        "subject": rendered.subject,
        // The engine writes a slash as `&#x2f;`: the same in HTML, and what the stored mail always said.
        "content": rendered.html.map(|h| h.replace("&#x2f;", "/")),
        "reference_entity": "Booking",
        "reference_id": bid.to_string(),
        "ics": invite,
    }));
    k.insert_tx(ctx, tx, "Communication", mail).await?;
    Ok(true)
}

/// Tells the staff member in-app, in the words of the notice's template: unless they made the change themselves.
pub(crate) async fn tell_staff(k: &Kernel, ctx: &Ctx, tx: &mut Tx, b: &Doc, note: StaffNote) -> Result<bool> {
    let tenant = ctx.tenant;
    let Some(email) = s(b, "staff_email") else { return Ok(false) };
    let user: Option<Uuid> = sqlx::query_scalar("select id from app_user where tenant_id = $1 and lower(email::text) = lower($2) and is_active")
        .bind(tenant)
        .bind(email)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(user) = user else { return Ok(false) };
    // A reminder goes to the person who made the booking too: they still want to be reminded.
    if note != StaffNote::Reminder && id(b, "updated_by") == Some(user) {
        return Ok(false);
    }
    let zone: Option<String> = match id(b, "staff") {
        Some(staff) => sqlx::query_scalar("select time_zone from e_booking_staff where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(staff)
            .fetch_optional(&mut **tx)
            .await?
            .flatten(),
        None => None,
    };
    let tz = zone.as_deref().and_then(calendar::zone).unwrap_or(Tz::UTC);
    let at = time(b, "start_time").map(|t| when(t, tz)).unwrap_or_default();
    let mut values = json!({"when": at});
    if note == StaffNote::Reminder {
        // A class session is one reminder, however many are booked: its seats are what its teacher sees.
        let session = s(b, "session_key");
        let seats: i64 =
            sqlx::query_scalar("select coalesce(sum(seats), 0)::bigint from e_booking where tenant_id = $1 and session_key = $2 and status = 'Confirmed'")
                .bind(tenant)
                .bind(session)
                .fetch_one(&mut **tx)
                .await?;
        values["class"] = json!(session != s(b, "id"));
        values["seats"] = json!(seats);
    }
    let m = Message {
        template: staff_template(note).into(),
        entity: Some("Booking".into()),
        record: id(b, "id"),
        to: vec![Recipient::user(user)],
        channels: vec!["inapp".into()],
        values: values.as_object().cloned().unwrap_or_default(),
        ..Default::default()
    };
    Ok(messages::send(k, tx, ctx, &m).await?.iter().any(|x| x.state == "delivered"))
}

/// One change notice per notice version.
async fn change_one(k: &Kernel, ctx: &Ctx, bid: Uuid) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let b = match k.get_tx(ctx, &mut tx, "Booking", bid, true).await {
        Ok(b) => b,
        Err(Error::NotFound(_)) => return Ok(0),
        Err(e) => return Err(e),
    };
    let (version, notified) = (int(&b, "notice_version"), int(&b, "notified_version"));
    if version <= notified {
        tx.commit().await?;
        return Ok(0);
    }
    let key = version.to_string();
    let mut sent = 0;
    if claim(&mut tx, ctx.tenant, bid, "change", &key, "pending").await? {
        let st = settings::load(&mut tx, ctx.tenant).await?;
        let (guest, staff) = change_messages(s(&b, "status").unwrap_or_default(), s(&b, "last_change").unwrap_or_default(), notified == 0);
        let mut channels = Vec::new();
        if let Some(g) = guest
            && email_guest(k, ctx, &mut tx, &b, g, &st).await?
        {
            channels.push("email");
        }
        if let Some(n) = staff
            && tell_staff(k, ctx, &mut tx, &b, n).await?
        {
            channels.push("in_app");
        }
        set_channel(&mut tx, ctx.tenant, bid, "change", &key, &channels).await?;
        sent = channels.len();
    }
    k.set_values_tx(ctx, &mut tx, "Booking", bid, obj(json!({"notified_version": version}))).await?;
    tx.commit().await?;
    Ok(sent)
}

/// Reminders due for one booking: the nearest offset is sent, offsets already passed are claimed
/// as skipped (a guest never gets a stale "tomorrow" after "in an hour").
async fn remind_one(k: &Kernel, ctx: &Ctx, bid: Uuid, start: Instant, due: &[i64]) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let b = match k.get_tx(ctx, &mut tx, "Booking", bid, true).await {
        Ok(b) => b,
        Err(Error::NotFound(_)) => return Ok(0),
        Err(e) => return Err(e),
    };
    if s(&b, "status") != Some("Confirmed") || time(&b, "start_time") != Some(start) {
        tx.commit().await?;
        return Ok(0);
    }
    let mut due = due.to_vec();
    due.sort_unstable();
    let key = |m: i64| format!("{m}@{}", start.timestamp());
    let mut sent = 0;
    let st = settings::load(&mut tx, ctx.tenant).await?;
    if let Some(nearest) = due.first().copied()
        && claim(&mut tx, ctx.tenant, bid, "reminder", &key(nearest), "pending").await?
    {
        let channels: &[&str] = if email_guest(k, ctx, &mut tx, &b, Guest::Reminder, &st).await? { &["email"] } else { &[] };
        set_channel(&mut tx, ctx.tenant, bid, "reminder", &key(nearest), channels).await?;
        sent = channels.len();
    }
    for m in due.iter().skip(1) {
        claim(&mut tx, ctx.tenant, bid, "reminder", &key(*m), "skipped").await?;
    }
    if st.remind_staff
        && let Some(staff) = id(&b, "staff")
    {
        sent += remind_staff(k, ctx, &mut tx, &b, staff, start, &due).await?;
    }
    tx.commit().await?;
    Ok(sent)
}

/// The claim key of a staff reminder: its offset and the start it is for (a moved booking is reminded again).
fn staff_key(minutes: i64, start: Instant) -> String {
    format!("staff:{minutes}@{}", start.timestamp())
}

/// Reminds the booked staff member, once per appointment or class session, offset and start. The staff row is
/// locked while we look for another booking of the session that was already counted, so two rounds cannot both
/// find none. `due` is sorted, nearest first; the further-out offsets are claimed as skipped, as for the guest.
async fn remind_staff(k: &Kernel, ctx: &Ctx, tx: &mut Tx, b: &Doc, staff: Uuid, start: Instant, due: &[i64]) -> Result<usize> {
    let (Some(&nearest), Some(bid)) = (due.first(), id(b, "id")) else { return Ok(0) };
    calendar::lock_staff(tx, ctx.tenant, &[staff]).await?;
    let key = staff_key(nearest, start);
    let told: bool = sqlx::query_scalar(
        "select exists (select 1 from bookings_notice n join e_booking o on o.tenant_id = n.tenant_id and o.id = n.booking_id \
         where n.tenant_id = $1 and n.kind = 'reminder' and n.key = $2 and o.session_key = $3 and o.id <> $4)",
    )
    .bind(ctx.tenant)
    .bind(&key)
    .bind(s(b, "session_key"))
    .bind(bid)
    .fetch_one(&mut **tx)
    .await?;
    let mut sent = 0;
    if told {
        claim(tx, ctx.tenant, bid, "reminder", &key, "skipped").await?;
    } else if claim(tx, ctx.tenant, bid, "reminder", &key, "pending").await? {
        let channels: &[&str] = if tell_staff(k, ctx, tx, b, StaffNote::Reminder).await? { &["in_app"] } else { &[] };
        set_channel(tx, ctx.tenant, bid, "reminder", &key, channels).await?;
        sent = channels.len();
    }
    for m in due.iter().skip(1) {
        claim(tx, ctx.tenant, bid, "reminder", &staff_key(*m, start), "skipped").await?;
    }
    Ok(sent)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Round {
    pub notices: usize,
    pub reminders: usize,
    pub expired: usize,
    pub promoted: usize,
    pub purged: u64,
    /// Bookings whose working calendar closed (or opened again) at their time.
    pub closed: usize,
    /// Unpaid deposits whose hold ran out, and what followed (see `payments::upkeep`).
    pub deposits: usize,
}

/// One maintenance round for a tenant. Steps run independently; the first error is returned after
/// all of them ran.
pub async fn run(k: &Kernel, tenant: Uuid, schema: Arc<Schema>, now: Instant) -> Result<Round> {
    let ctx = Ctx::system(tenant, schema);
    let mut round = Round::default();
    let mut first_error: Option<Error> = None;
    let mut note = |step: &str, e: Error| {
        tracing::warn!(%tenant, step, "bookings maintenance failed: {e}");
        first_error.get_or_insert(e);
    };
    match changes(k, &ctx).await {
        Ok(n) => round.notices = n,
        Err(e) => note("notices", e),
    }
    match reminders(k, &ctx, now).await {
        Ok(n) => round.reminders = n,
        Err(e) => note("reminders", e),
    }
    match expire_waitlists(k, &ctx, now).await {
        Ok(n) => round.expired = n,
        Err(e) => note("waitlist expiry", e),
    }
    match sweep_waitlists(k, &ctx, now).await {
        Ok(n) => round.promoted = n,
        Err(e) => note("waitlist promotion", e),
    }
    match purge(k, &ctx, now).await {
        Ok(n) => round.purged = n,
        Err(e) => note("purge", e),
    }
    match crate::payments::upkeep(k, &ctx, now).await {
        Ok(n) => round.deposits = n,
        Err(e) => note("deposits", e),
    }
    match closed_clashes(k, &ctx, now).await {
        Ok(n) => round.closed = n,
        Err(e) => note("opening hours", e),
    }
    first_error.map_or(Ok(round), Err)
}

async fn changes(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let ids: Vec<Uuid> =
        sqlx::query_scalar("select id from e_booking where tenant_id = $1 and notice_version > notified_version order by updated_at, id limit 200")
            .bind(ctx.tenant)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let mut sent = 0;
    for bid in ids {
        sent += with_retry(|| change_one(k, ctx, bid)).await?;
    }
    Ok(sent)
}

async fn reminders(k: &Kernel, ctx: &Ctx, now: Instant) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let st = settings::load(&mut tx, ctx.tenant).await?;
    // The furthest offset any service (or the workspace) reminds at bounds the bookings read.
    let services: Option<i64> =
        sqlx::query_scalar("select max(minutes_before) from e_booking_service__reminders where tenant_id = $1").bind(ctx.tenant).fetch_one(&mut *tx).await?;
    let Some(furthest) = st.reminders.first().copied().max(services) else {
        tx.commit().await?;
        return Ok(0);
    };
    let rows: Vec<(Uuid, Instant, Instant, Uuid)> = sqlx::query_as(
        "select id, start_time, coalesce(scheduled_at, created_at), service from e_booking where tenant_id = $1 and status = 'Confirmed' \
         and start_time > $2 and start_time <= $2 + make_interval(mins => $3) order by start_time, id limit 1000",
    )
    .bind(ctx.tenant)
    .bind(now)
    .bind(furthest as i32)
    .fetch_all(&mut *tx)
    .await?;
    let mut offsets: HashMap<Uuid, Vec<i64>> = HashMap::new();
    for (_, _, _, service) in &rows {
        if !offsets.contains_key(service) {
            let svc = calendar::service(&mut tx, ctx.tenant, *service).await?;
            offsets.insert(*service, svc.map_or_else(|| st.reminders.clone(), |svc| st.for_service(&svc).reminders));
        }
    }
    tx.commit().await?;
    let mut sent = 0;
    for (bid, start, scheduled, service) in rows {
        let due: Vec<i64> = offsets[&service]
            .iter()
            .copied()
            .filter(|m| {
                let at = start - Duration::minutes(*m);
                at <= now && at >= scheduled
            })
            .collect();
        if !due.is_empty() {
            sent += with_retry(|| remind_one(k, ctx, bid, start, &due)).await?;
        }
    }
    Ok(sent)
}

/// Waitlist places of classes that have started are cancelled (the guest is told).
async fn expire_waitlists(k: &Kernel, ctx: &Ctx, now: Instant) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let ids: Vec<Uuid> =
        sqlx::query_scalar("select id from e_booking where tenant_id = $1 and status = 'Waitlisted' and start_time <= $2 order by start_time, id limit 200")
            .bind(ctx.tenant)
            .bind(now)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let mut expired = 0;
    for bid in ids {
        expired += with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let b = k.get_tx(ctx, &mut tx, "Booking", bid, true).await?;
            if s(&b, "status") != Some("Waitlisted") {
                tx.commit().await?;
                return Ok(0);
            }
            booking::cancel(k, ctx, &mut tx, &b, Some("No seat became free before the class started."), false, false).await?;
            tx.commit().await?;
            Ok(1)
        })
        .await?;
    }
    Ok(expired)
}

/// Promotes waitlists whose sessions have room again (a capacity raised, a promotion skipped
/// because the row was busy).
async fn sweep_waitlists(k: &Kernel, ctx: &Ctx, now: Instant) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let sessions: Vec<(String, Uuid)> = sqlx::query_as(
        "select session_key, service from e_booking where tenant_id = $1 and status = 'Waitlisted' and start_time > $2 \
         group by session_key, service order by min(start_time) limit 100",
    )
    .bind(ctx.tenant)
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut promoted = 0;
    for (key, service) in sessions {
        promoted += with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let Some(svc) = calendar::service(&mut tx, ctx.tenant, service).await? else {
                tx.commit().await?;
                return Ok(0);
            };
            let st = settings::load(&mut tx, ctx.tenant).await?;
            let n = booking::promote(k, ctx, &mut tx, &svc, &st, &key).await?;
            tx.commit().await?;
            Ok(n)
        })
        .await?;
    }
    Ok(promoted)
}

/// Active bookings ahead whose working calendars (the staff member's, the business's) no longer open
/// at their time are flagged `closed_clash`: kept, never moved or cancelled. A flagged one whose time
/// opens again is cleared. Staff on weekly hours alone are not checked (as before calendars).
async fn closed_clashes(k: &Kernel, ctx: &Ctx, now: Instant) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let st = settings::load(&mut tx, ctx.tenant).await?;
    let rows: Vec<(Uuid, Uuid, Instant, Instant, bool)> = sqlx::query_as(
        "select id, staff, start_time, end_time, coalesce(closed_clash, false) from e_booking where tenant_id = $1 \
         and status in ('Pending', 'Confirmed') and start_time > $2 order by start_time, id limit 2000",
    )
    .bind(ctx.tenant)
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;
    let Some(window) = rows.first().zip(rows.iter().map(|r| r.3).max()).map(|(a, end)| Span::new(a.2, end)) else {
        tx.commit().await?;
        return Ok(0);
    };
    let mut ids: Vec<Uuid> = rows.iter().map(|r| r.1).collect();
    ids.sort();
    ids.dedup();
    let business = calendar::business_open(&mut tx, ctx.tenant, &st, window).await?;
    let mut changed = 0;
    for s in calendar::staff(&mut tx, ctx.tenant, &ids).await? {
        let Some(open) = calendar::staff_open(&mut tx, ctx.tenant, &s, &business, window).await? else { continue };
        for (bid, _, start, end, flagged) in rows.iter().filter(|r| r.1 == s.id) {
            let closed = !open.iter().any(|w| w.contains(&Span::new(*start, *end)));
            if closed != *flagged {
                k.set_values_tx(ctx, &mut tx, "Booking", *bid, obj(json!({"closed_clash": closed}))).await?;
                changed += 1;
            }
        }
    }
    tx.commit().await?;
    Ok(changed)
}

/// Guest links of bookings over for 30 days, and claims of bookings over for 60 days.
async fn purge(k: &Kernel, ctx: &Ctx, now: Instant) -> Result<u64> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let tokens = sqlx::query(
        "delete from bookings_token t using e_booking b where t.tenant_id = $1 and b.tenant_id = t.tenant_id and b.id = t.booking_id \
         and b.end_time < $2 - make_interval(days => $3)",
    )
    .bind(ctx.tenant)
    .bind(now)
    .bind(token::LINK_DAYS as i32)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let claims = sqlx::query(
        "delete from bookings_notice n using e_booking b where n.tenant_id = $1 and b.tenant_id = n.tenant_id and b.id = n.booking_id \
         and b.end_time < $2 - interval '60 days'",
    )
    .bind(ctx.tenant)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(tokens + claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_notice_describes_the_booking_as_it_is() {
        use Guest as G;
        use StaffNote as S;
        assert_eq!(change_messages("Confirmed", "Booked", true), (Some(G::Confirmed), Some(S::New)));
        assert_eq!(change_messages("Confirmed", "Rescheduled", true), (Some(G::Confirmed), Some(S::New)), "moved before anyone was told");
        assert_eq!(change_messages("Pending", "Booked", true), (Some(G::Requested), Some(S::AwaitsConfirmation)));
        assert_eq!(change_messages("Waitlisted", "Booked", true), (Some(G::Waitlisted), None));
        assert_eq!(change_messages("Cancelled", "Cancelled", true), (None, None), "never announced, now gone");
        assert_eq!(change_messages("Confirmed", "Confirmed", false), (Some(G::Confirmed), None));
        assert_eq!(change_messages("Confirmed", "Rescheduled", false), (Some(G::Rescheduled), Some(S::Rescheduled)));
        assert_eq!(change_messages("Pending", "Rescheduled", false), (Some(G::Rescheduled), Some(S::Rescheduled)));
        assert_eq!(change_messages("Confirmed", "Promoted", false), (Some(G::Confirmed), Some(S::SeatFilled)));
        assert_eq!(change_messages("Pending", "Promoted", false), (Some(G::Requested), Some(S::AwaitsConfirmation)));
        assert_eq!(change_messages("Cancelled", "Cancelled", false), (Some(G::Cancelled), Some(S::Cancelled)));
        assert_eq!(change_messages("Completed", "Booked", false), (None, None));
        assert_eq!(change_messages("Expired", "Expired", false), (Some(G::Expired), Some(S::Expired)));
        assert_eq!(change_messages("Expired", "Expired", true), (Some(G::Expired), None), "released before it was announced");
        assert_eq!(change_messages("Expired", "Refunded", false), (Some(G::Refunded), None));
        assert_eq!(change_messages("Confirmed", "Restored", false), (Some(G::Confirmed), Some(S::New)));
    }

    #[test]
    fn each_kind_has_its_template_and_a_deposit_changes_the_first() {
        use Guest as G;
        let all = [G::Requested, G::Confirmed, G::Waitlisted, G::Rescheduled, G::Cancelled, G::Reminder, G::Expired, G::Refunded];
        let keys: Vec<&str> = all.iter().map(|g| template_for(*g, false)).collect();
        assert_eq!(keys.len(), 8);
        assert!(keys.iter().all(|k| k.starts_with("bookings.guest_")));
        assert_eq!(template_for(G::Requested, true), "bookings.guest_deposit_due");
        assert!(all.iter().filter(|g| **g != G::Requested).all(|g| template_for(*g, true) == template_for(*g, false)), "only a request asks for the deposit");
        assert_eq!(deposit_text(&json!({"amount": "300", "currency": "INR"})), "300.00 INR");
    }

    #[test]
    fn a_staff_reminder_is_claimed_by_offset_and_start() {
        let start = "2026-11-02T04:30:00Z".parse::<Instant>().unwrap();
        assert_eq!(staff_key(60, start), format!("staff:60@{}", start.timestamp()));
        assert_ne!(staff_key(60, start), staff_key(1440, start));
        assert_ne!(staff_key(60, start), staff_key(60, start + Duration::minutes(30)), "a moved booking is reminded again");
        assert_eq!(staff_template(StaffNote::Reminder), "bookings.staff_reminder");
    }

    #[test]
    fn a_booking_is_a_calendar_event() {
        let b = obj(json!({
            "id": "0192d3c4-0000-7000-8000-000000000001", "number": "BKG-2026-00007", "service_name": "Massage", "staff_name": "Asha",
            "start_time": "2026-11-02T04:30:00.000Z", "end_time": "2026-11-02T05:30:00.000Z",
            "location_type": "Video", "location": "https://meet.example.test/x?a=1&b=2",
        }));
        let event = booking_event(&b, "Massage with Asha".into(), "CONFIRMED").unwrap();
        assert_eq!(event.uid, "0192d3c4-0000-7000-8000-000000000001@bookings.ferrum");
        assert!(event.description.unwrap().contains("Video: https://meet.example.test"));
    }
}
