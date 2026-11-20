//! The engine's inputs, read from the database, and the row locks that make booking safe under
//! concurrency: whatever takes time (a booking, a reschedule, a waitlist promotion) first locks its
//! candidate staff rows, then their resources, each in id order — one global order, so two
//! requests cannot deadlock — and only then reads the calendar and decides. Seats of class sessions
//! and resource capacity are counted under those locks; the exclusion constraint on `e_booking`
//! guards staff time once more in the database itself.

use crate::Instant;
use crate::engine::{self, Booked, Hours, Span, World};
use crate::settings::Settings;
use chrono::Duration;
use chrono_tz::Tz;
use ferrum_kernel::{Error, Result, Tx};
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Question {
    pub question: String,
    pub answer_type: String,
    pub options: Vec<String>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub id: Uuid,
    pub name: String,
    pub active: bool,
    pub description: Option<String>,
    pub duration: i64,
    pub before: i64,
    pub after: i64,
    pub interval: Option<i64>,
    pub capacity: i64,
    pub price: Decimal,
    pub item: Option<Uuid>,
    pub location_type: String,
    pub location: Option<String>,
    pub staff: Vec<Uuid>,
    /// Where it is offered; empty: every location.
    pub locations: Vec<Uuid>,
    /// Each staff member's (priority, weight) for this service; missing: (0, 1).
    pub rank: HashMap<Uuid, (i64, i64)>,
    pub resources: Vec<Uuid>,
    pub questions: Vec<Question>,
    /// Times in a row a guest may book it from the booking page (0 or 1: once).
    pub guest_repeats: i64,
    /// Its own booking rules; empty ones are Booking Settings' (see `Settings::for_service`).
    pub rules: ServiceRules,
    /// A class offered only at its scheduled sessions.
    pub sessions_only: bool,
}

/// A service's own booking rules, each `None` (or empty) when it follows Booking Settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceRules {
    pub min_notice_minutes: Option<i64>,
    pub max_advance_days: Option<i64>,
    pub cancellation_window_hours: Option<i64>,
    pub reschedule_window_hours: Option<i64>,
    pub approval_required: Option<bool>,
    /// Reminder offsets in minutes; empty: the workspace's.
    pub reminders: Vec<i64>,
}

impl ServiceInfo {
    pub fn group(&self) -> bool {
        self.capacity > 1
    }

    pub fn engine(&self, st: &Settings) -> engine::Service {
        engine::Service {
            id: self.id,
            duration: Duration::minutes(self.duration),
            before: Duration::minutes(self.before),
            after: Duration::minutes(self.after),
            capacity: self.capacity,
            interval: self.interval.map(Duration::minutes).unwrap_or(st.interval),
            sessions_only: self.sessions_only,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StaffInfo {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub tz: Tz,
    pub active: bool,
    pub bio: Option<String>,
    pub hours: Vec<Hours>,
    pub feed_version: i64,
    /// Their working calendar (Admin → Calendars): when set, it replaces the weekly hours.
    pub calendar: Option<String>,
    /// Where they work; `None`: every location.
    pub location: Option<Uuid>,
}

/// An active place the business takes bookings at.
#[derive(Debug, Clone)]
pub struct LocationInfo {
    pub id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub zone: Option<Tz>,
}

/// The workspace's active locations, by name.
pub async fn locations(tx: &mut Tx, tenant: Uuid) -> Result<Vec<LocationInfo>> {
    let rows: Vec<(Uuid, String, Option<String>, Option<String>)> =
        sqlx::query_as("select id, location_name, address, time_zone from e_booking_location where tenant_id = $1 and active order by location_name, id")
            .bind(tenant)
            .fetch_all(&mut **tx)
            .await?;
    let blank = |v: Option<String>| v.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    Ok(rows.into_iter().map(|(id, name, address, tz)| LocationInfo { id, name, address: blank(address), zone: tz.as_deref().and_then(zone) }).collect())
}

/// Where a booking of `offered_at` (the service's locations; empty: everywhere) takes place, among the
/// workspace's active locations `all`: the one asked for, else the named staff member's, else the only one
/// offered. `None` when the workspace has no location: exactly as before locations existed.
pub fn place_among(all: &[Uuid], offered_at: &[Uuid], requested: Option<Uuid>, staff_location: Option<Uuid>) -> Result<Option<Uuid>> {
    if all.is_empty() {
        return Ok(None);
    }
    let offered: Vec<Uuid> = if offered_at.is_empty() { all.to_vec() } else { all.iter().copied().filter(|l| offered_at.contains(l)).collect() };
    if let Some(r) = requested {
        return if offered.contains(&r) { Ok(Some(r)) } else { Err(Error::field("booking_location", "this service is not offered there")) };
    }
    if let Some(l) = staff_location.filter(|l| offered.contains(l)) {
        return Ok(Some(l));
    }
    match offered[..] {
        [one] => Ok(Some(one)),
        [] => Err(Error::field("booking_location", "this service is not offered at any open location")),
        _ => Err(Error::field("booking_location", "choose a location")),
    }
}

/// [`place_among`] for `svc`, with the workspace's active locations and the named staff member's.
pub async fn place(tx: &mut Tx, tenant: Uuid, svc: &ServiceInfo, requested: Option<Uuid>, staff: Option<Uuid>) -> Result<Option<Uuid>> {
    let all: Vec<Uuid> = locations(tx, tenant).await?.into_iter().map(|l| l.id).collect();
    if all.is_empty() {
        return Ok(None);
    }
    let staff_location = match staff {
        Some(s) => self::staff(tx, tenant, &[s]).await?.first().and_then(|p| p.location),
        None => None,
    };
    place_among(&all, &svc.locations, requested, staff_location)
}

pub fn zone(name: &str) -> Option<Tz> {
    name.trim().parse::<Tz>().ok()
}

type ServiceRow = (
    Uuid,
    String,
    bool,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<Decimal>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

pub async fn service(tx: &mut Tx, tenant: Uuid, id: Uuid) -> Result<Option<ServiceInfo>> {
    let row: Option<ServiceRow> = sqlx::query_as(
        "select id, service_name, active, description, duration_minutes, buffer_before_minutes, buffer_after_minutes, slot_interval_minutes, \
         capacity, price, item, location_type, location_details, guest_repeat_max from e_booking_service where tenant_id = $1 and id = $2",
    )
    .bind(tenant)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((id, name, active, description, duration, before, after, interval, capacity, price, item, location_type, location, guest_repeats)) = row else {
        return Ok(None);
    };
    // Rows saved before priority and weight existed hold null in both: no priority, a weight of 1.
    let ranked: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "select staff, coalesce(priority, 0)::bigint, coalesce(weight, 1)::bigint from e_booking_service__staff \
         where tenant_id = $1 and parent_id = $2 and staff is not null order by idx",
    )
    .bind(tenant)
    .bind(id)
    .fetch_all(&mut **tx)
    .await?;
    let staff: Vec<Uuid> = ranked.iter().map(|r| r.0).collect();
    let mut rank = HashMap::new();
    for (s, priority, weight) in ranked {
        rank.entry(s).or_insert((priority, weight.max(1)));
    }
    let resources: Vec<Uuid> =
        sqlx::query_scalar("select resource from e_booking_service__resources where tenant_id = $1 and parent_id = $2 and resource is not null order by idx")
            .bind(tenant)
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    let locations: Vec<Uuid> =
        sqlx::query_scalar("select location from e_booking_service__locations where tenant_id = $1 and parent_id = $2 and location is not null order by idx")
            .bind(tenant)
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    let questions: Vec<(String, Option<String>, Option<String>, bool)> = sqlx::query_as(
        "select question, answer_type, options, required from e_booking_service__questions where tenant_id = $1 and parent_id = $2 order by idx",
    )
    .bind(tenant)
    .bind(id)
    .fetch_all(&mut **tx)
    .await?;
    let (min_notice_minutes, max_advance_days, cancellation_window_hours, reschedule_window_hours, approval, sessions_only): (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        "select min_notice_minutes, max_advance_days, cancellation_window_hours, reschedule_window_hours, approval_required, \
         coalesce(sessions_only, false) from e_booking_service where tenant_id = $1 and id = $2",
    )
    .bind(tenant)
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let reminders: Vec<i64> =
        sqlx::query_scalar("select minutes_before from e_booking_service__reminders where tenant_id = $1 and parent_id = $2 and minutes_before > 0")
            .bind(tenant)
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    let rules = ServiceRules {
        min_notice_minutes,
        max_advance_days,
        cancellation_window_hours,
        reschedule_window_hours,
        approval_required: approval.as_deref().and_then(|a| match a {
            "Yes" => Some(true),
            "No" => Some(false),
            _ => None,
        }),
        reminders,
    };
    Ok(Some(ServiceInfo {
        rules,
        sessions_only,
        id,
        name,
        active,
        description,
        duration: duration.unwrap_or(60).max(1),
        before: before.unwrap_or(0).max(0),
        after: after.unwrap_or(0).max(0),
        interval: interval.filter(|i| *i > 0),
        capacity: capacity.unwrap_or(1).max(1),
        price: price.unwrap_or_default(),
        item,
        location_type: location_type.unwrap_or_else(|| "In Person".into()),
        location: location.filter(|l| !l.trim().is_empty()),
        guest_repeats: guest_repeats.unwrap_or(0).max(0),
        staff: dedup(staff),
        locations: dedup(locations),
        rank,
        resources: dedup(resources),
        questions: questions
            .into_iter()
            .map(|(question, answer_type, options, required)| Question {
                question,
                answer_type: answer_type.unwrap_or_else(|| "Text".into()),
                options: options.unwrap_or_default().lines().map(str::trim).filter(|o| !o.is_empty()).map(str::to_string).collect(),
                required,
            })
            .collect(),
    }))
}

/// Active services, by name (the public page lists them).
pub async fn active_services(tx: &mut Tx, tenant: Uuid) -> Result<Vec<ServiceInfo>> {
    let ids: Vec<Uuid> = sqlx::query_scalar("select id from e_booking_service where tenant_id = $1 and active order by service_name, id limit 500")
        .bind(tenant)
        .fetch_all(&mut **tx)
        .await?;
    let mut out = Vec::with_capacity(ids.len());
    // ponytail: one query set per service; a workspace offers tens of services, not thousands.
    for id in ids {
        if let Some(s) = service(tx, tenant, id).await? {
            out.push(s);
        }
    }
    Ok(out)
}

fn dedup(ids: Vec<Uuid>) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter(|i| seen.insert(*i)).collect()
}

/// Staff members with their working hours, in the order of `ids` (unknown ids are left out).
pub async fn staff(tx: &mut Tx, tenant: Uuid, ids: &[Uuid]) -> Result<Vec<StaffInfo>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    type Row = (Uuid, String, String, Option<String>, bool, Option<String>, Option<i64>, Option<String>, Option<Uuid>);
    let rows: Vec<Row> = sqlx::query_as(
        "select id, display_name, email, time_zone, active, bio, feed_version, calendar, location from e_booking_staff where tenant_id = $1 and id = any($2)",
    )
    .bind(tenant)
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;
    let hours: Vec<(Uuid, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "select parent_id, weekday, start_time, end_time from e_booking_staff__working_hours where tenant_id = $1 and parent_id = any($2) order by parent_id, idx",
    )
    .bind(tenant)
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut by_staff: HashMap<Uuid, Vec<Hours>> = HashMap::new();
    for (staff, day, from, to) in hours {
        let parsed = (|| {
            Some(Hours { weekday: engine::weekday(day.as_deref()?)?, from: engine::parse_hhmm(from.as_deref()?)?, to: engine::parse_hhmm(to.as_deref()?)? })
        })();
        if let Some(h) = parsed {
            by_staff.entry(staff).or_default().push(h);
        }
    }
    let mut found: HashMap<Uuid, StaffInfo> = rows
        .into_iter()
        .map(|(id, name, email, tz, active, bio, feed_version, calendar, location)| {
            let info = StaffInfo {
                id,
                name,
                email: email.to_lowercase(),
                tz: tz.as_deref().and_then(zone).unwrap_or(Tz::UTC),
                active,
                bio,
                hours: by_staff.remove(&id).unwrap_or_default(),
                feed_version: feed_version.unwrap_or(1),
                calendar: calendar.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
                location,
            };
            (id, info)
        })
        .collect();
    Ok(ids.iter().filter_map(|i| found.remove(i)).collect())
}

/// The open stretches of the platform calendar `cal` inside `window`, from its own functions
/// (`next_open`, `working_between`), so its hours, weekends, holidays and dated exceptions are one source.
/// A stretch ends where the open time counted from its start stops being continuous.
// ponytail: the end of a stretch is found by bisection over `working_between`; about twenty calls per
// stretch, fine for the weeks a slot query covers. Upgrade: ask the platform to expose its open windows.
pub fn open_spans(cal: &ferrum_kernel::Calendar, window: Span) -> Vec<Span> {
    let mut out = Vec::new();
    let mut at = window.start;
    while at < window.end {
        let Ok(start) = cal.next_open(at) else { break };
        if start >= window.end {
            break;
        }
        let continuous = |secs: i64| cal.working_between(start, start + Duration::seconds(secs), &[]) == Ok(Duration::seconds(secs));
        // Gallop in doubling days to a first break, then bisect: a stretch is usually well under a day.
        let (mut lo, left, mut probe) = (0, (window.end - start).num_seconds(), 86_400);
        let mut hi = loop {
            let p = probe.min(left);
            if continuous(p) {
                lo = p;
                if p == left {
                    break p;
                }
                probe *= 2;
            } else {
                break p - 1;
            }
        };
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            if continuous(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo == 0 {
            break;
        }
        let end = start + Duration::seconds(lo);
        out.push(Span::new(start, end));
        at = end;
    }
    out
}

/// A working calendar's open time near `window` (Admin → Calendars), exceptions included. A calendar that
/// is missing or unusable is closed: it never opens time nobody set.
pub async fn calendar_open(tx: &mut Tx, tenant: Uuid, name: &str, window: Span) -> Result<Vec<Span>> {
    let Some(cal) = ferrum_kernel::load_calendar(tx, tenant, name).await? else { return Ok(vec![]) };
    if cal.problem().is_some() {
        return Ok(vec![]);
    }
    Ok(open_spans(&cal, window))
}

/// The business's open time near `window`: its calendar's; `None` when it named none (nothing is closed
/// for everyone).
pub async fn business_open(tx: &mut Tx, tenant: Uuid, st: &Settings, window: Span) -> Result<Option<Vec<Span>>> {
    match &st.calendar {
        Some(name) => Ok(Some(calendar_open(tx, tenant, name, window).await?)),
        None => Ok(None),
    }
}

/// A staff member's working time near `window` when calendars decide it (theirs, the business's):
/// their calendar's open time, or their weekly hours, inside the business's open time. `None`: their
/// weekly hours alone, exactly as without calendars.
pub async fn staff_open(tx: &mut Tx, tenant: Uuid, s: &StaffInfo, business: &Option<Vec<Span>>, window: Span) -> Result<Option<Vec<Span>>> {
    let own = match &s.calendar {
        Some(name) => Some(calendar_open(tx, tenant, name, window).await?),
        None => None,
    };
    Ok(match (own, business) {
        (None, None) => None,
        (own, Some(b)) => Some(engine::intersect(own.unwrap_or_else(|| engine::working_spans(s.tz, &s.hours, window)), b.clone())),
        (own, None) => own,
    })
}

/// Locks staff rows in id order for the rest of the transaction.
pub async fn lock_staff(tx: &mut Tx, tenant: Uuid, ids: &[Uuid]) -> Result<()> {
    if !ids.is_empty() {
        sqlx::query("select id from e_booking_staff where tenant_id = $1 and id = any($2) order by id for update")
            .bind(tenant)
            .bind(ids)
            .fetch_all(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Locks resource rows in id order (after the staff rows).
pub async fn lock_resources(tx: &mut Tx, tenant: Uuid, ids: &[Uuid]) -> Result<()> {
    if !ids.is_empty() {
        sqlx::query("select id from e_booking_resource where tenant_id = $1 and id = any($2) order by id for update")
            .bind(tenant)
            .bind(ids)
            .fetch_all(&mut **tx)
            .await?;
    }
    Ok(())
}

type SessionRow = (String, Uuid, Uuid, Instant, Instant, Instant, Instant, Option<i64>, Option<Uuid>, Option<Uuid>);

type BookedRow = (Uuid, Uuid, Uuid, Option<Uuid>, Instant, Instant, Instant, Instant, String, Option<i64>);

/// The engine's view of `svc` for `staff` around `around`: their time off and every active
/// booking of theirs or on the service's resources (two days of margin, so buffers and whole local
/// days are covered). `exclude` leaves out the booking being moved.
/// `place`: the location the booking is at; only its rooms (and rooms of every location) count, and its opening
/// calendar replaces the business's.
#[allow(clippy::too_many_arguments)]
pub async fn world(
    tx: &mut Tx,
    tenant: Uuid,
    svc: &ServiceInfo,
    staff: &[StaffInfo],
    st: &Settings,
    around: Span,
    exclude: Option<Uuid>,
    place: Option<Uuid>,
) -> Result<World> {
    let from = around.start - Duration::days(2);
    let to = around.end + Duration::days(2);
    let staff_ids: Vec<Uuid> = staff.iter().map(|s| s.id).collect();
    let range = Span::new(from, to);
    // A room of another location has no unit here: a service that needs rooms finds none free at a location without one.
    let resources: Vec<(Uuid, Option<i64>, Option<String>)> = if svc.resources.is_empty() {
        vec![]
    } else {
        sqlx::query_as(
            "select id, case when location is null or location = $3 then capacity else 0 end, calendar from e_booking_resource \
             where tenant_id = $1 and id = any($2) and active",
        )
        .bind(tenant)
        .bind(&svc.resources)
        .bind(place)
        .fetch_all(&mut **tx)
        .await?
    };
    let off: Vec<(Uuid, Instant, Instant)> = sqlx::query_as(
        "select staff, from_time, to_time from e_staff_time_off where tenant_id = $1 and staff = any($2) and to_time > $3 and from_time < $4 \
         and from_time is not null and to_time is not null",
    )
    .bind(tenant)
    .bind(&staff_ids)
    .bind(from)
    .bind(to)
    .fetch_all(&mut **tx)
    .await?;
    let elsewhere: Vec<(Uuid, Instant, Instant)> = sqlx::query_as(
        "select staff, from_time, to_time from bookings_outside_busy where tenant_id = $1 and staff = any($2) and to_time > $3 and from_time < $4",
    )
    .bind(tenant)
    .bind(&staff_ids)
    .bind(from)
    .bind(to)
    .fetch_all(&mut **tx)
    .await?;
    let booked: Vec<BookedRow> = sqlx::query_as(
        "select id, staff, service, resource, start_time, end_time, block_start, block_end, session_key, seats from e_booking \
         where tenant_id = $1 and status in ('Pending', 'Confirmed') and block_end > $3 and block_start < $4 \
         and (staff = any($2) or resource = any($5)) and id <> $6",
    )
    .bind(tenant)
    .bind(&staff_ids)
    .bind(from)
    .bind(to)
    .bind(&svc.resources)
    .bind(exclude.unwrap_or_default())
    .fetch_all(&mut **tx)
    .await?;
    // Scheduled sessions of every service: they hold their teacher's time and their room.
    let sessions: Vec<SessionRow> = sqlx::query_as(
        "select session_key, staff, service, start_time, end_time, block_start, block_end, capacity, resource, booking_location from e_booking_session \
         where tenant_id = $1 and status = 'Scheduled' and block_end > $3 and block_start < $4 and (staff = any($2) or resource = any($5))",
    )
    .bind(tenant)
    .bind(&staff_ids)
    .bind(from)
    .bind(to)
    .bind(&svc.resources)
    .fetch_all(&mut **tx)
    .await?;
    // A location with an opening calendar of its own opens and closes for everyone booked there.
    let own: Option<String> = match place {
        Some(p) => sqlx::query_scalar("select calendar from e_booking_location where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(p)
            .fetch_optional(&mut **tx)
            .await?
            .flatten()
            .map(|c: String| c.trim().to_string())
            .filter(|c| !c.is_empty()),
        None => None,
    };
    let business = match own {
        Some(name) => Some(calendar_open(tx, tenant, &name, range).await?),
        None => business_open(tx, tenant, st, range).await?,
    };
    let mut working = Vec::new();
    for s in staff.iter().filter(|s| s.active) {
        working.push(engine::Staff {
            id: s.id,
            tz: s.tz,
            hours: s.hours.clone(),
            time_off: off.iter().filter(|o| o.0 == s.id).map(|o| Span::new(o.1, o.2)).collect(),
            busy: elsewhere.iter().filter(|o| o.0 == s.id).map(|o| Span::new(o.1, o.2)).collect(),
            open: staff_open(tx, tenant, s, &business, range).await?,
            priority: svc.rank.get(&s.id).map_or(0, |r| r.0),
            weight: svc.rank.get(&s.id).map_or(1, |r| r.1),
        });
    }
    // In the service's order of preference.
    let mut usable = Vec::new();
    for (id, capacity, cal) in svc.resources.iter().filter_map(|r| resources.iter().find(|x| x.0 == *r)) {
        let open = match cal.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(name) => Some(calendar_open(tx, tenant, name, range).await?),
            None => None,
        };
        usable.push(engine::Resource { id: *id, capacity: capacity.unwrap_or(1).max(0), open });
    }
    Ok(World {
        service: svc.engine(st),
        staff: working,
        resources: usable,
        booked: booked
            .into_iter()
            .map(|(id, staff, service, resource, start, end, block_start, block_end, session, seats)| Booked {
                id,
                staff,
                service,
                resource,
                span: Span::new(start, end),
                block: Span::new(block_start, block_end),
                session,
                seats: seats.unwrap_or(1),
            })
            .collect(),
        rules: Default::default(),
        place,
        sessions: sessions
            .into_iter()
            .map(|(key, staff, service, start, end, block_start, block_end, capacity, resource, place)| engine::Declared {
                key,
                staff,
                service,
                span: Span::new(start, end),
                block: Span::new(block_start, block_end),
                capacity: capacity.filter(|c| *c > 0),
                resource,
                place,
            })
            .collect(),
    })
}

#[cfg(test)]
mod place_tests {
    use super::place_among;
    use uuid::Uuid;

    fn field(r: ferrum_kernel::Result<Option<Uuid>>) -> String {
        match r {
            Err(ferrum_kernel::Error::Validation(f)) => format!("{}: {}", f[0].field, f[0].message),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn a_booking_is_placed_by_the_rules_in_order() {
        let (north, south, east) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let all = [north, south, east];
        assert_eq!(place_among(&[], &[], None, None).unwrap(), None, "no locations: as before");
        assert_eq!(place_among(&[], &[north], Some(south), Some(east)).unwrap(), None, "no active location: nothing to place");
        assert_eq!(place_among(&all, &[], Some(south), Some(north)).unwrap(), Some(south), "the one asked for");
        assert_eq!(field(place_among(&all, &[north], Some(south), None)), "booking_location: this service is not offered there");
        assert_eq!(place_among(&all, &[north, south], None, Some(south)).unwrap(), Some(south), "the named staff member's");
        assert_eq!(place_among(&all, &[north], None, Some(south)).unwrap(), Some(north), "theirs is not offered: the only one that is");
        assert_eq!(place_among(&all, &[east], None, None).unwrap(), Some(east), "the only one offered");
        assert_eq!(field(place_among(&all, &[], None, None)), "booking_location: choose a location");
        assert_eq!(field(place_among(&all, &[Uuid::from_u128(9)], None, None)), "booking_location: this service is not offered at any open location");
    }
}
