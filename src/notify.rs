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
use crate::ics::{self, Event};
use crate::settings::{self, Settings};
use crate::{Instant, int, obj, time, token, when};
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use ferrum_domain::util::{id, s};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx, with_retry};
use ferrum_meta::Schema;
use serde_json::json;
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
}

/// What the staff member is told in-app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaffNote {
    New,
    AwaitsConfirmation,
    Rescheduled,
    Cancelled,
    SeatFilled,
}

/// The messages for a change notice, from the booking's current state and whether anything was
/// sent before (`first`): the first notice always describes the booking as it is now, so a booking
/// made and moved within a minute is announced once, at its new time.
pub fn change_messages(status: &str, last_change: &str, first: bool) -> (Option<Guest>, Option<StaffNote>) {
    match (status, first) {
        ("Cancelled", true) => (None, None),
        ("Cancelled", false) => (Some(Guest::Cancelled), Some(StaffNote::Cancelled)),
        ("Waitlisted", true) => (Some(Guest::Waitlisted), None),
        ("Pending", true) => (Some(Guest::Requested), Some(StaffNote::AwaitsConfirmation)),
        ("Confirmed", true) => (Some(Guest::Confirmed), Some(StaffNote::New)),
        ("Pending" | "Confirmed", false) => match last_change {
            "Rescheduled" => (Some(Guest::Rescheduled), Some(StaffNote::Rescheduled)),
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

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
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

/// Subject and HTML body of a guest email.
pub fn compose(kind: Guest, b: &Doc, tz: Tz, manage: &str, ics_url: &str) -> (String, String) {
    let service = s(b, "service_name").unwrap_or("Booking");
    let at = time(b, "start_time").map(|t| when(t, tz)).unwrap_or_default();
    let (subject, lead) = match kind {
        Guest::Requested => (format!("Booking request received: {service}, {at}"), "We have your booking request and will confirm it shortly."),
        Guest::Confirmed => (format!("Booking confirmed: {service}, {at}"), "Your booking is confirmed."),
        Guest::Waitlisted => {
            (format!("On the waitlist: {service}, {at}"), "The class is full, so you are on its waitlist. We will write as soon as a seat frees up.")
        }
        Guest::Rescheduled => (format!("Booking moved: {service}, {at}"), "Your booking has a new time."),
        Guest::Cancelled => (format!("Booking cancelled: {service}, {at}"), "Your booking is cancelled."),
        Guest::Reminder => (format!("Reminder: {service}, {at}"), "This is a reminder of your booking."),
    };
    let row = |label: &str, value: &str| format!("<tr><td style=\"padding:2px 12px 2px 0;color:#57606a\">{label}</td><td>{}</td></tr>", html_escape(value));
    let mut rows = vec![row("What", service), row("When", &at)];
    if let Some(end) = time(b, "end_time") {
        rows.push(row("Until", &end.with_timezone(&tz).format("%H:%M").to_string()));
    }
    if let Some(staff) = s(b, "staff_name") {
        rows.push(row("With", staff));
    }
    if kind != Guest::Cancelled {
        let place = match (s(b, "location_type"), s(b, "location")) {
            (Some(kind), Some(l)) => format!("{kind}: {l}"),
            (Some(kind), None) => kind.to_string(),
            (None, Some(l)) => l.to_string(),
            (None, None) => String::new(),
        };
        if !place.is_empty() {
            rows.push(row("Where", &place));
        }
    }
    if int(b, "seats") > 1 {
        rows.push(row("Seats", &int(b, "seats").to_string()));
    }
    rows.push(row("Reference", s(b, "number").unwrap_or_default()));
    if kind == Guest::Cancelled
        && let Some(reason) = s(b, "cancellation_reason")
    {
        rows.push(row("Reason", reason));
    }
    let links = if kind == Guest::Cancelled {
        String::new()
    } else {
        format!("<p><a href=\"{}\">View, reschedule or cancel</a> &middot; <a href=\"{}\">Add to calendar</a></p>", html_escape(manage), html_escape(ics_url))
    };
    let html = format!(
        "<p>Hello {},</p><p>{lead}</p><table style=\"border-collapse:collapse\">{}</table>{links}",
        html_escape(s(b, "guest_name").unwrap_or("there")),
        rows.join("")
    );
    (subject, html)
}

/// Emails the guest through the Mail app (with a fresh self-service link and, for confirmations,
/// moves and cancellations, a calendar invitation). `false` when there is no Mail app or address.
async fn email_guest(k: &Kernel, ctx: &Ctx, tx: &mut Tx, b: &Doc, kind: Guest, st: &Settings) -> Result<bool> {
    let Some(to) = s(b, "guest_email") else { return Ok(false) };
    if !mail_installed(ctx) {
        return Ok(false);
    }
    let bid = id(b, "id").ok_or_else(|| Error::internal("booking without id"))?;
    let root = settings::site_root(tx, ctx.tenant).await?;
    let token = token::mint(tx, ctx.tenant, bid).await?;
    let manage = st.manage_link(&root, &token);
    let ics_url = st.absolute(&format!("{root}/manage/{token}/booking.ics"));
    let (subject, html) = compose(kind, b, guest_zone(b, ctx), &manage, &ics_url);
    let summary = format!("{} with {}", s(b, "service_name").unwrap_or("Booking"), s(b, "staff_name").unwrap_or("us"));
    let invite = match kind {
        Guest::Confirmed | Guest::Rescheduled => booking_event(b, summary, "CONFIRMED").map(|e| ics::calendar(Some("REQUEST"), None, &[e])),
        Guest::Cancelled => booking_event(b, summary, "CANCELLED").map(|e| ics::calendar(Some("CANCEL"), None, &[e])),
        _ => None,
    };
    let mail = obj(json!({
        "channel": "Email",
        "direction": "Sent",
        "recipients": to,
        "subject": subject,
        "content": html,
        "reference_entity": "Booking",
        "reference_id": bid.to_string(),
        "ics": invite,
    }));
    k.insert_tx(ctx, tx, "Communication", mail).await?;
    Ok(true)
}

/// Tells the staff member in-app — unless they made the change themselves.
async fn tell_staff(tx: &mut Tx, tenant: Uuid, b: &Doc, note: StaffNote) -> Result<bool> {
    let Some(email) = s(b, "staff_email") else { return Ok(false) };
    let user: Option<Uuid> = sqlx::query_scalar("select id from app_user where tenant_id = $1 and lower(email::text) = lower($2) and is_active")
        .bind(tenant)
        .bind(email)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(user) = user else { return Ok(false) };
    if id(b, "updated_by") == Some(user) {
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
    let service = s(b, "service_name").unwrap_or("Booking");
    let guest = s(b, "guest_name").unwrap_or("a guest");
    let at = time(b, "start_time").map(|t| when(t, tz)).unwrap_or_default();
    let title = match note {
        StaffNote::New => format!("New booking: {service} with {guest}"),
        StaffNote::AwaitsConfirmation => format!("To confirm: {service} with {guest}"),
        StaffNote::Rescheduled => format!("Booking moved: {service} with {guest}"),
        StaffNote::Cancelled => format!("Booking cancelled: {service} with {guest}"),
        StaffNote::SeatFilled => format!("Waitlist seat filled: {service} with {guest}"),
    };
    let mut body = format!("{at} · {}", s(b, "number").unwrap_or_default());
    if note == StaffNote::Cancelled
        && let Some(reason) = s(b, "cancellation_reason")
    {
        body.push_str(&format!(" · {reason}"));
    }
    ferrum_kernel::notify::send(tx, tenant, user, &title, Some(&body), Some("Booking"), id(b, "id")).await?;
    Ok(true)
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
            && tell_staff(&mut tx, ctx.tenant, &b, n).await?
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
    if let Some(nearest) = due.first().copied()
        && claim(&mut tx, ctx.tenant, bid, "reminder", &key(nearest), "pending").await?
    {
        let st = settings::load(&mut tx, ctx.tenant).await?;
        let channels: &[&str] = if email_guest(k, ctx, &mut tx, &b, Guest::Reminder, &st).await? { &["email"] } else { &[] };
        set_channel(&mut tx, ctx.tenant, bid, "reminder", &key(nearest), channels).await?;
        sent = channels.len();
    }
    for m in due.iter().skip(1) {
        claim(&mut tx, ctx.tenant, bid, "reminder", &key(*m), "skipped").await?;
    }
    tx.commit().await?;
    Ok(sent)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Round {
    pub notices: usize,
    pub reminders: usize,
    pub expired: usize,
    pub promoted: usize,
    pub purged: u64,
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
    let Some(furthest) = st.reminders.first().copied() else {
        tx.commit().await?;
        return Ok(0);
    };
    let rows: Vec<(Uuid, Instant, Instant)> = sqlx::query_as(
        "select id, start_time, coalesce(scheduled_at, created_at) from e_booking where tenant_id = $1 and status = 'Confirmed' \
         and start_time > $2 and start_time <= $2 + make_interval(mins => $3) order by start_time, id limit 1000",
    )
    .bind(ctx.tenant)
    .bind(now)
    .bind(furthest as i32)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut sent = 0;
    for (bid, start, scheduled) in rows {
        let due: Vec<i64> = st
            .reminders
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
            booking::cancel(k, ctx, &mut tx, &b, Some("No seat became free before the class started."), false).await?;
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
    }

    #[test]
    fn guest_emails_escape_what_guests_typed() {
        let b = obj(json!({
            "id": "0192d3c4-0000-7000-8000-000000000001", "number": "BKG-2026-00007", "service_name": "Massage", "staff_name": "Asha",
            "guest_name": "<script>alert(1)</script>", "start_time": "2026-11-02T04:30:00.000Z", "end_time": "2026-11-02T05:30:00.000Z",
            "location_type": "Video", "location": "https://meet.example.test/x?a=1&b=2", "seats": 2, "cancellation_reason": "Sick & sorry",
        }));
        let kolkata: Tz = "Asia/Kolkata".parse().unwrap();
        let (subject, html) = compose(Guest::Confirmed, &b, kolkata, "https://spa.test/m?token=t&x=1", "/ics");
        assert_eq!(subject, "Booking confirmed: Massage, Mon 2 Nov 2026, 10:00 (Asia/Kolkata)");
        assert!(!html.contains("<script>") && html.contains("&lt;script&gt;"));
        assert!(html.contains("Video: https://meet.example.test/x?a=1&amp;b=2"));
        assert!(html.contains("href=\"https://spa.test/m?token=t&amp;x=1\""));
        assert!(html.contains(">11:00<") && html.contains(">2<") && html.contains("BKG-2026-00007"));
        let (_, cancelled) = compose(Guest::Cancelled, &b, kolkata, "m", "i");
        assert!(cancelled.contains("Sick &amp; sorry") && !cancelled.contains("href="), "no self-service links on a cancelled booking");
        let event = booking_event(&b, "Massage with Asha".into(), "CONFIRMED").unwrap();
        assert_eq!(event.uid, "0192d3c4-0000-7000-8000-000000000001@bookings.ferrum");
        assert!(event.description.unwrap().contains("Video: https://meet.example.test"));
    }
}
