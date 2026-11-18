//! Class sessions scheduled before anyone books them (`BookingSession`): what scheduling one checks,
//! the guard on edits, and when one may be deleted. A session's key is its own id (`s/{id}`), fixed
//! for life; every seat booked into it carries that key.
//!
//! A session and a booking live in two tables, so no constraint spans them: scheduling a session,
//! changing its seats and deleting it lock the teacher's `e_booking_staff` row first, as every
//! booking does (`calendar::lock_staff`), and decide from a calendar read after the lock.

use crate::calendar;
use crate::engine::{Span, Why};
use crate::settings;
use crate::{doc_id, obj, time, ts};
use async_trait::async_trait;
use chrono::Duration;
use ferrum_domain::util::{id, s};
use ferrum_kernel::{Doc, DocHook, Error, HookCtx, Result};
use serde_json::{Value, json};
use uuid::Uuid;

/// The key every seat of a scheduled session shares.
pub fn key_of(session: Uuid) -> String {
    format!("s/{}", session.simple())
}

/// The engine's refusal as a booking's is told: taken time or room is a conflict, anything else the start's.
pub(crate) fn refused(why: Why, note: &str) -> Error {
    let message = format!("{}{note}", why.message());
    if matches!(why, Why::Busy | Why::NoResource) { Error::Conflict(message) } else { Error::field("start_time", message) }
}

/// A new session: its service gives classes, its teacher gives the service and is free then, its room is
/// free; the derived fields follow.
async fn schedule(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    let tenant = hc.ctx.tenant;
    let sid = id(doc, "service").ok_or_else(|| Error::field("service", "is required"))?;
    let svc = calendar::service(hc.tx, tenant, sid).await?.ok_or_else(|| Error::field("service", "does not exist"))?;
    if !svc.group() {
        return Err(Error::field("service", format!("{} is not a group class", svc.name)));
    }
    if !svc.active {
        return Err(Error::field("service", format!("{} is not taking bookings", svc.name)));
    }
    let staff = id(doc, "staff").ok_or_else(|| Error::field("staff", "is required"))?;
    if !svc.staff.contains(&staff) {
        return Err(Error::field("staff", format!("does not provide {}", svc.name)));
    }
    let start = time(doc, "start_time").ok_or_else(|| Error::field("start_time", "is required"))?;
    let room = id(doc, "resource");
    if room.is_some_and(|r| !svc.resources.contains(&r)) {
        return Err(Error::field("resource", format!("is not one of the rooms of {}", svc.name)));
    }
    calendar::lock_staff(hc.tx, tenant, &[staff]).await?;
    calendar::lock_resources(hc.tx, tenant, &room.into_iter().collect::<Vec<_>>()).await?;
    let people = calendar::staff(hc.tx, tenant, &[staff]).await?;
    let Some(teacher) = people.first().filter(|p| p.active).cloned() else { return Err(Error::field("staff", "is not taking bookings")) };
    let st = settings::load(hc.tx, tenant).await?.for_service(&svc);
    let end = start + Duration::minutes(svc.duration);
    let block = Span::new(start - Duration::minutes(svc.before), end + Duration::minutes(svc.after));
    let world = calendar::world(hc.tx, tenant, &svc, &people, &st, Span::new(start, end), None).await?;
    let key = key_of(doc_id(doc)?);
    if let Err(why) = world.free_at(staff, start, Some(&key)) {
        // Seats booked before at that time share a key made of service, teacher and start: not this session's.
        let earlier = world.booked.iter().any(|b| b.staff == staff && b.service == svc.id && b.span.start == start);
        let note =
            if why == Why::Busy && earlier { ": seats already booked for that class then stay as they are; a session cannot take them over" } else { "" };
        return Err(refused(why, note));
    }
    if let Some(r) = room {
        if !world.resources.iter().any(|x| x.id == r) {
            return Err(Error::field("resource", "is not an active room or resource"));
        }
        if !world.room_free(r, Span::new(start, end), block, None) {
            return Err(Error::Conflict("that room is taken then".into()));
        }
    }
    doc.extend(obj(json!({
        "service_name": svc.name,
        "staff_name": teacher.name,
        "staff_email": teacher.email,
        "end_time": ts(end),
        "block_start": ts(block.start),
        "block_end": ts(block.end),
        "session_key": key,
        "status": "Scheduled",
    })));
    Ok(())
}

/// Seats taken in the session: its Pending and Confirmed bookings.
pub(crate) async fn taken(hc: &mut HookCtx<'_>, key: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "select coalesce(sum(coalesce(seats, 1)), 0)::bigint from e_booking where tenant_id = $1 and session_key = $2 and status in ('Pending', 'Confirmed')",
    )
    .bind(hc.ctx.tenant)
    .bind(key)
    .fetch_one(&mut **hc.tx)
    .await?)
}

/// A change of seats: never below the seats already taken, counted under the teacher's lock.
async fn reseat(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let tenant = hc.ctx.tenant;
    let before: Option<Option<i64>> = sqlx::query_scalar("select capacity from e_booking_session where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(doc_id(doc)?)
        .fetch_optional(&mut **hc.tx)
        .await?;
    let now = doc.get("capacity").and_then(Value::as_i64);
    if before.is_none_or(|b| b == now) {
        return Ok(());
    }
    if let Some(staff) = id(doc, "staff") {
        calendar::lock_staff(hc.tx, tenant, &[staff]).await?;
    }
    let seats = match now {
        Some(n) => n,
        None => match id(doc, "service") {
            Some(svc) => calendar::service(hc.tx, tenant, svc).await?.map_or(1, |s| s.capacity),
            None => 1,
        },
    };
    let taken = taken(hc, s(doc, "session_key").unwrap_or_default()).await?;
    if seats < taken {
        return Err(Error::field("capacity", format!("{taken} seats are already taken")));
    }
    Ok(())
}

pub struct SessionHook;

#[async_trait]
impl DocHook for SessionHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.contains_key("created_at") { reseat(hc, doc).await } else { schedule(hc, doc).await }
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if let Some(old) = old
            && !hc.ctx.is_system
        {
            for f in ["service", "staff", "start_time", "resource", "status"] {
                if old.get(f) != doc.get(f) {
                    return Err(Error::field(f, "changes only through the session's actions (Move, Cancel Session)"));
                }
            }
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if s(doc, "status") != Some("Scheduled") {
            return Ok(());
        }
        // Under the teacher's lock: no booking can enter the session while it goes.
        if let Some(staff) = id(doc, "staff") {
            calendar::lock_staff(hc.tx, hc.ctx.tenant, &[staff]).await?;
        }
        let booked: i64 =
            sqlx::query_scalar("select count(*) from e_booking where tenant_id = $1 and session_key = $2 and status in ('Pending', 'Confirmed', 'Waitlisted')")
                .bind(hc.ctx.tenant)
                .bind(s(doc, "session_key").unwrap_or_default())
                .fetch_one(&mut **hc.tx)
                .await?;
        if booked > 0 {
            return Err(Error::invalid(format!("{booked} booking(s) are in this session: cancel its bookings first")));
        }
        Ok(())
    }
}
