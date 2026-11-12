//! A video meeting per booking (audit BK-48). A Video service that states no link of its own gets a
//! meeting of the Mail, Chat and Calendar app (Comms) for each booking once it is confirmed, hosted
//! by the booked staff member. The guest's own join link is written to the booking's `location`, so
//! the guest page, the confirmation, the reminders and the calendar entry all carry it; a booking
//! moved moves its meeting (the link stays the same) and one cancelled or expired ends it.
//!
//! Everything here is for a booking that has no link of its own to show: without Comms, without a
//! meeting server, without the workspace's public address, or when the service states its link, a
//! booking is exactly what it was. A meeting that cannot be made never stops a booking: the
//! failure is logged and the booking goes on without it.

use crate::calendar;
use crate::time;
use chrono::Utc;
use ferrum_app_comms::meetings::{self, MeetingError, NewMeeting};
use ferrum_domain::util::{id, s};
use ferrum_kernel::{Doc, HookCtx, Result};
use serde_json::json;
use uuid::Uuid;

/// Brings the booking's meeting in step with the booking being saved: `doc` is the booking as it
/// will be stored. Saving again changes nothing.
pub async fn sync(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    if s(doc, "location_type") != Some("Video") {
        return Ok(());
    }
    let (Some(booking), Some(service)) = (id(doc, "id"), id(doc, "service")) else { return Ok(()) };
    let tenant = hc.ctx.tenant;
    let status = s(doc, "status").unwrap_or_default();
    if !matches!(status, "Confirmed" | "Cancelled" | "Expired") {
        return Ok(());
    }
    // The service's own link (or number) is what its guests are given.
    let own = calendar::service(hc.tx, tenant, service).await?.is_some_and(|svc| svc.location.as_deref().is_some_and(|l| !l.trim().is_empty()));
    if own {
        return Ok(());
    }
    let outcome = match status {
        "Confirmed" => confirmed(hc, doc, booking).await,
        _ => ended(hc, booking).await,
    };
    match outcome {
        Ok(()) | Err(MeetingError::NotInstalled | MeetingError::NotConfigured) => {}
        Err(e) => tracing::warn!(%tenant, %booking, "no video meeting for this booking: {e}"),
    }
    Ok(())
}

async fn confirmed(hc: &mut HookCtx<'_>, doc: &mut Doc, booking: Uuid) -> std::result::Result<(), MeetingError> {
    let tenant = hc.ctx.tenant;
    meetings::available(hc.kernel, tenant).await?;
    let (Some(start), Some(end)) = (time(doc, "start_time"), time(doc, "end_time")) else { return Ok(()) };
    if end <= Utc::now() {
        return Ok(());
    }
    let meeting = match meetings::find(hc.kernel, tenant, "Booking", booking).await?.filter(|m| m.ended_at.is_none()) {
        Some(m) if (m.starts_at, m.ends_at) == (start, end) => m,
        Some(m) => meetings::reschedule(hc.kernel, tenant, m.id, start, end).await?,
        None => {
            let host = host(hc, doc).await?;
            let title = s(doc, "service_name").unwrap_or("Appointment");
            meetings::create(hc.kernel, tenant, NewMeeting { entity: "Booking", record: booking, title, starts_at: start, ends_at: end, host }).await?
        }
    };
    // A guest without an address still gets a link: it is for this meeting only either way.
    let email = s(doc, "guest_email").map_or_else(|| format!("guest-{}@bookings.invalid", booking.simple()), str::to_string);
    let link = meetings::guest_link(hc.kernel, tenant, meeting.id, s(doc, "guest_name").unwrap_or_default(), &email).await?;
    doc.insert("location".into(), json!(link.url));
    Ok(())
}

async fn ended(hc: &mut HookCtx<'_>, booking: Uuid) -> std::result::Result<(), MeetingError> {
    let tenant = hc.ctx.tenant;
    if let Some(m) = meetings::find(hc.kernel, tenant, "Booking", booking).await?.filter(|m| m.ended_at.is_none()) {
        meetings::end(hc.kernel, tenant, m.id).await?;
    }
    Ok(())
}

/// The booked staff member's user, else whoever is making the change.
async fn host(hc: &mut HookCtx<'_>, doc: &Doc) -> std::result::Result<Uuid, MeetingError> {
    let by_email: Option<Uuid> = match s(doc, "staff_email") {
        Some(email) => {
            sqlx::query_scalar("select id from app_user where tenant_id = $1 and is_active and lower(email::text) = lower($2)")
                .bind(hc.ctx.tenant)
                .bind(email)
                .fetch_optional(&mut **hc.tx)
                .await?
        }
        None => None,
    };
    Ok(by_email.unwrap_or(hc.ctx.user))
}
