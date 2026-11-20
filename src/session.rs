//! Class sessions scheduled before anyone books them (`BookingSession`): what scheduling one checks,
//! the guard on edits, and when one may be deleted. A session's key is its own id (`s/{id}`), fixed
//! for life; every seat booked into it carries that key.
//!
//! A session and a booking live in two tables, so no constraint spans them: scheduling a session,
//! changing its seats and deleting it lock the teacher's `e_booking_staff` row first, as every
//! booking does (`calendar::lock_staff`), and decide from a calendar read after the lock.

use crate::engine::{Span, Why};
use crate::{booking, calendar, series, settings};
use crate::{doc_id, obj, time, ts};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use ferrum_domain::util::{elevated, id, s, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx};
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
    let place = calendar::place(hc.tx, tenant, &svc, id(doc, "booking_location"), Some(staff)).await?;
    calendar::lock_staff(hc.tx, tenant, &[staff]).await?;
    calendar::lock_resources(hc.tx, tenant, &room.into_iter().collect::<Vec<_>>()).await?;
    let people = calendar::staff(hc.tx, tenant, &[staff]).await?;
    let Some(teacher) = people.first().filter(|p| p.active).cloned() else { return Err(Error::field("staff", "is not taking bookings")) };
    if place.is_some() && teacher.location.is_some_and(|l| Some(l) != place) {
        return Err(Error::field("staff", "does not work at that location"));
    }
    let st = settings::load(hc.tx, tenant).await?.for_service(&svc);
    let end = start + Duration::minutes(svc.duration);
    let block = Span::new(start - Duration::minutes(svc.before), end + Duration::minutes(svc.after));
    let world = calendar::world(hc.tx, tenant, &svc, &people, &st, Span::new(start, end), None, place).await?;
    let key = key_of(doc_id(doc)?);
    if let Err(why) = world.free_at(staff, start, Some(&key)) {
        // Seats booked before at that time share a key made of service, teacher and start: not this session's.
        let earlier = world.booked.iter().any(|b| b.staff == staff && b.service == svc.id && b.span.start == start);
        let note =
            if why == Why::Busy && earlier { ": seats already booked for that class then stay as they are; a session cannot take them over" } else { "" };
        return Err(refused(why, note));
    }
    if let Some(r) = room {
        if !world.resources.iter().any(|x| x.id == r && x.capacity > 0) {
            return Err(Error::field("resource", "is not an active room or resource at this location"));
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
        "booking_location": place.map(|l| l.to_string()),
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

/// The most bookings one session can hold (1000 seats, and their waitlist): a guard, not a path.
const MOST_BOOKINGS: u32 = 2000;

/// A reason as guests read it: no control characters, at most 1000 characters.
fn clean(reason: Option<&str>) -> Option<String> {
    reason.map(|r| r.chars().filter(|c| !c.is_control() || *c == '\n').take(1000).collect::<String>().trim().to_string()).filter(|r| !r.is_empty())
}

/// The session's bookings with `statuses`, oldest first, each locked; a booking whose status changed while waiting
/// for its lock is left out.
async fn bookings_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, key: &str, statuses: &[&str]) -> Result<Vec<Doc>> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("select id from e_booking where tenant_id = $1 and session_key = $2 and status = any($3) order by created_at, id limit $4")
            .bind(ctx.tenant)
            .bind(key)
            .bind(statuses.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .bind(i64::from(MOST_BOOKINGS) + 1)
            .fetch_all(&mut **tx)
            .await?;
    if ids.len() > MOST_BOOKINGS as usize {
        return Err(Error::invalid("this session has too many bookings to change at once"));
    }
    let mut out = Vec::new();
    for b in ids {
        let row = k.get_tx(ctx, tx, "Booking", b, true).await?;
        if statuses.contains(&s(&row, "status").unwrap_or_default()) {
            out.push(row);
        }
    }
    Ok(out)
}

/// `cancel_session {reason?}`: the session and every booking in it, waitlist places first (so no freed seat is offered
/// to them); each seat as a business cancellation (a paid deposit comes back whole), each guest told once.
async fn cancel_session(hc: &mut HookCtx<'_>, doc: &Doc, reason: Option<&str>) -> Result<Value> {
    if s(doc, "status") != Some("Scheduled") {
        return Err(Error::invalid("this session is already cancelled"));
    }
    let (k, engine) = (hc.kernel, elevated(hc.ctx));
    let session = doc_id(doc)?;
    let key = s(doc, "session_key").ok_or_else(|| Error::internal("a scheduled session without its key"))?.to_string();
    calendar::lock_staff(hc.tx, hc.ctx.tenant, &id(doc, "staff").into_iter().collect::<Vec<_>>()).await?;
    let reason = clean(reason);
    k.update_tx(&engine, hc.tx, "BookingSession", session, obj(json!({"status": "Cancelled", "cancellation_reason": reason})), None).await?;
    let mut cancelled = Vec::new();
    for statuses in [&["Waitlisted"][..], &["Pending", "Confirmed"][..]] {
        for row in bookings_of(k, &engine, hc.tx, &key, statuses).await? {
            booking::cancel(k, &engine, hc.tx, &row, reason.as_deref(), false, true).await?;
            cancelled.push(doc_id(&row)?.to_string());
        }
    }
    Ok(json!({"session": session, "cancelled": cancelled}))
}

/// A link given as an action parameter: empty is none.
fn link_param(params: &Value, name: &str) -> Result<Option<Uuid>> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) if v.trim().is_empty() => Ok(None),
        Some(Value::String(v)) => v.trim().parse().map(Some).map_err(|_| Error::field(name, "is not an id")),
        Some(_) => Err(Error::field(name, "is not an id")),
    }
}

/// `move {start_time, staff?, resource?}`: the session and every seat in it to a new time, teacher or room, decided
/// once for the whole session under the old and new teachers' locks; a seat is told once, a waitlist place moves
/// silently. Where it already is: nothing changes.
async fn move_session(hc: &mut HookCtx<'_>, doc: &Doc, params: &Value) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let (k, engine) = (hc.kernel, elevated(hc.ctx));
    let session = doc_id(doc)?;
    let key = s(doc, "session_key").ok_or_else(|| Error::internal("a scheduled session without its key"))?.to_string();
    let start = params
        .get("start_time")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::field("start_time", "is required"))
        .and_then(|v| ferrum_kernel::value::parse_datetime(v).map_err(|m| Error::field("start_time", m)))?;
    if start <= Utc::now() {
        return Err(Error::field("start_time", "a session moves to a time ahead"));
    }
    let svc = calendar::service(hc.tx, tenant, id(doc, "service").ok_or_else(|| Error::internal("a session without its service"))?)
        .await?
        .ok_or_else(|| Error::internal("a session's service is gone"))?;
    let old_teacher = id(doc, "staff").ok_or_else(|| Error::internal("a session without its teacher"))?;
    let teacher = link_param(params, "staff")?.unwrap_or(old_teacher);
    if !svc.staff.contains(&teacher) {
        return Err(Error::field("staff", format!("does not provide {}", svc.name)));
    }
    // The session's room, else the one its seats hold.
    let held: Option<Uuid> = match id(doc, "resource") {
        Some(r) => Some(r),
        None => sqlx::query_scalar(
            "select resource from e_booking where tenant_id = $1 and session_key = $2 and status in ('Pending', 'Confirmed') and resource is not null limit 1",
        )
        .bind(tenant)
        .bind(&key)
        .fetch_optional(&mut **hc.tx)
        .await?,
    };
    let named = link_param(params, "resource")?;
    let room = named.or(held);
    if room.is_some_and(|r| !svc.resources.contains(&r)) {
        return Err(Error::field("resource", format!("is not one of the rooms of {}", svc.name)));
    }
    if time(doc, "start_time") == Some(start) && teacher == old_teacher && room == held {
        return Ok(json!({"session": session, "moved": 0}));
    }
    calendar::lock_staff(hc.tx, tenant, &[old_teacher, teacher]).await?;
    calendar::lock_resources(hc.tx, tenant, &held.into_iter().chain(room).collect::<Vec<_>>()).await?;
    let people = calendar::staff(hc.tx, tenant, &[teacher]).await?;
    let Some(person) = people.first().filter(|p| p.active).cloned() else { return Err(Error::field("staff", "is not taking bookings")) };
    // A session stays at its location: a teacher who works elsewhere is not one it moves to.
    let place = id(doc, "booking_location");
    if place.is_some() && person.location.is_some_and(|l| Some(l) != place) {
        return Err(Error::field("staff", "does not work at that location"));
    }
    let st = settings::load(hc.tx, tenant).await?.for_service(&svc);
    let end = start + Duration::minutes(svc.duration);
    let block = Span::new(start - Duration::minutes(svc.before), end + Duration::minutes(svc.after));
    let mut world = calendar::world(hc.tx, tenant, &svc, &people, &st, Span::new(start, end), None, place).await?;
    world.sessions.retain(|d| d.key != key);
    world.booked.retain(|b| b.session != key);
    world.free_at(teacher, start, Some(&key)).map_err(|why| refused(why, ""))?;
    if let Some(r) = room {
        if !world.resources.iter().any(|x| x.id == r && x.capacity > 0) {
            return Err(Error::field("resource", "is not an active room or resource"));
        }
        if !world.room_free(r, Span::new(start, end), block, Some(&key)) {
            return Err(Error::Conflict("that room is taken then".into()));
        }
    }
    let place = obj(json!({
        "staff": teacher.to_string(),
        "staff_name": person.name,
        "staff_email": person.email,
        "start_time": ts(start),
        "end_time": ts(end),
    }));
    let mut fixed = place.clone();
    fixed.extend(obj(json!({"block_start": ts(block.start), "block_end": ts(block.end), "resource": room.map(|r| r.to_string())})));
    let mut session_patch = fixed.clone();
    if id(doc, "resource").is_none() && named.is_none() {
        // A session that named no room keeps naming none: its seats hold the room.
        session_patch.remove("resource");
    }
    k.update_tx(&engine, hc.tx, "BookingSession", session, session_patch, None).await?;
    let mut moved = 0;
    for row in bookings_of(k, &engine, hc.tx, &key, &["Pending", "Confirmed"]).await? {
        let mut patch = fixed.clone();
        patch.extend(obj(json!({
            "scheduled_at": ts(Utc::now()),
            "last_change": "Rescheduled",
            "outside_clash": false,
            "outside_clash_since": null,
            "closed_clash": false,
        })));
        booking::update(k, &engine, hc.tx, &row, patch, true).await?;
        moved += 1;
    }
    for row in bookings_of(k, &engine, hc.tx, &key, &["Waitlisted"]).await? {
        booking::update(k, &engine, hc.tx, &row, place.clone(), false).await?;
    }
    Ok(json!({"session": session, "moved": moved}))
}

/// `repeat {every, count, skip_conflicts?}`: the later sessions of a timetable, each scheduled and checked as one
/// made by hand, at the same local time in the teacher's zone; whole or nothing unless asked to skip taken dates.
async fn repeat(hc: &mut HookCtx<'_>, doc: &Doc, params: &Value) -> Result<Value> {
    if s(doc, "status") != Some("Scheduled") {
        return Err(Error::invalid("only a scheduled session can be repeated"));
    }
    if s(doc, "series").is_some() {
        return Err(Error::invalid("this session is in a series already"));
    }
    let every = series::Every::parse(params.get("every").and_then(Value::as_str))?;
    let count = params.get("count").and_then(Value::as_u64).filter(|c| (2..=series::MAX_OCCURRENCES as u64).contains(c));
    let count = count.ok_or_else(|| Error::field("count", format!("must be 2 to {}, this session included", series::MAX_OCCURRENCES)))? as u32;
    let skip = params.get("skip_conflicts").is_some_and(|v| v.as_bool().unwrap_or(v.as_str() == Some("Yes")));
    let first = time(doc, "start_time").ok_or_else(|| Error::internal("a session without a start"))?;
    let staff = id(doc, "staff").ok_or_else(|| Error::internal("a session without its teacher"))?;
    let (k, engine) = (hc.kernel, elevated(hc.ctx));
    let tz = calendar::staff(hc.tx, hc.ctx.tenant, &[staff]).await?.first().map(|p| p.tz).ok_or_else(|| Error::internal("a session's teacher is gone"))?;
    let name = Uuid::now_v7().to_string();
    let (mut scheduled, mut skipped) = (Vec::new(), Vec::new());
    for date in series::dates(first.with_timezone(&tz).date_naive(), every, count).into_iter().skip(1) {
        let start = series::same_local_time(first, tz, date);
        let next = obj(json!({
            "service": doc.get("service"),
            "staff": staff.to_string(),
            "start_time": ts(start),
            "capacity": doc.get("capacity"),
            "resource": doc.get("resource"),
            "booking_location": doc.get("booking_location"),
            "series": name,
        }));
        // A refused date must not cost the others the transaction.
        sqlx::query("savepoint session_date").execute(&mut **hc.tx).await?;
        match k.insert_tx(&engine, hc.tx, "BookingSession", next).await {
            Ok(made) => scheduled.push(json!({"id": made.get("id"), "start_time": ts(start)})),
            Err(e @ (Error::Validation(_) | Error::Invalid(_) | Error::Conflict(_))) => {
                sqlx::query("rollback to savepoint session_date").execute(&mut **hc.tx).await?;
                let why = series::refusal(&e);
                if !skip {
                    return Err(Error::Invalid(format!(
                        "{date} cannot be scheduled: {why}. Nothing was repeated; choose another time, or skip the dates that are taken"
                    )));
                }
                skipped.push(json!({"date": date, "start_time": ts(start), "reason": why}));
            }
            Err(e) => return Err(e),
        }
    }
    if scheduled.is_empty() {
        return Err(Error::invalid("none of the later dates can be scheduled"));
    }
    k.set_values_tx(&sys(hc.ctx), hc.tx, "BookingSession", doc_id(doc)?, obj(json!({"series": name}))).await?;
    Ok(json!({"series": name, "scheduled": scheduled, "skipped": skipped}))
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
            for f in ["service", "staff", "start_time", "resource", "status", "booking_location"] {
                if old.get(f) != doc.get(f) {
                    return Err(Error::field(f, "changes only through the session's actions (Move, Cancel Session)"));
                }
            }
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        Ok(Some(match name {
            "cancel_session" => cancel_session(hc, doc, params.get("reason").and_then(Value::as_str)).await?,
            "move" => move_session(hc, doc, params).await?,
            "repeat" => repeat(hc, doc, params).await?,
            _ => return Ok(None),
        }))
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
