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
use ferrum_kernel::{Result, Tx};
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
    pub resources: Vec<Uuid>,
    pub questions: Vec<Question>,
    /// Times in a row a guest may book it from the booking page (0 or 1: once).
    pub guest_repeats: i64,
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
    let staff: Vec<Uuid> =
        sqlx::query_scalar("select staff from e_booking_service__staff where tenant_id = $1 and parent_id = $2 and staff is not null order by idx")
            .bind(tenant)
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    let resources: Vec<Uuid> =
        sqlx::query_scalar("select resource from e_booking_service__resources where tenant_id = $1 and parent_id = $2 and resource is not null order by idx")
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
    Ok(Some(ServiceInfo {
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
    let rows: Vec<(Uuid, String, String, Option<String>, bool, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "select id, display_name, email, time_zone, active, bio, feed_version, calendar from e_booking_staff where tenant_id = $1 and id = any($2)",
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
        .map(|(id, name, email, tz, active, bio, feed_version, calendar)| {
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
            };
            (id, info)
        })
        .collect();
    Ok(ids.iter().filter_map(|i| found.remove(i)).collect())
}

/// A working calendar's open time near `window` (Admin → Calendars), with the dated changes entered
/// in Booking Settings for it (with `business`, also those for the business as a whole). A calendar
/// that is missing or unusable is closed: it never opens time nobody set.
pub async fn calendar_open(tx: &mut Tx, tenant: Uuid, st: &Settings, name: &str, business: bool, window: Span) -> Result<Vec<Span>> {
    let Some(cal) = ferrum_kernel::load_calendar(tx, tenant, name).await? else { return Ok(vec![]) };
    let Some(tz) = zone(&cal.zone).filter(|_| cal.problem().is_none()) else { return Ok(vec![]) };
    let mut hours = Vec::new();
    for (i, day) in ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter().enumerate() {
        let Ok(weekday) = chrono::Weekday::try_from(i as u8) else { continue };
        if cal.weekends.iter().any(|w| w == day) {
            continue;
        }
        match &cal.hours {
            // Without hours of the day every hour of a business day is open.
            None => hours.push(Hours { weekday, from: 0, to: 1440 }),
            Some(h) => hours.extend(
                h.get(*day).into_iter().flatten().filter_map(|(a, b)| Some(Hours { weekday, from: engine::parse_hhmm(a)?, to: engine::parse_hhmm(b)? })),
            ),
        }
    }
    let holidays: Vec<_> = cal.holidays.iter().copied().collect();
    Ok(engine::calendar_spans(tz, &hours, &holidays, &st.dated_for(name, business), window))
}

/// The business's open time near `window`: its calendar's, or every hour less its dated closures;
/// `None` when it set neither (nothing is closed for everyone).
pub async fn business_open(tx: &mut Tx, tenant: Uuid, st: &Settings, window: Span) -> Result<Option<Vec<Span>>> {
    if let Some(name) = &st.calendar {
        return Ok(Some(calendar_open(tx, tenant, st, name, true, window).await?));
    }
    let dated = st.dated_for("", true);
    if dated.is_empty() {
        return Ok(None);
    }
    let all_day: Vec<Hours> = engine::WEEKDAYS.iter().filter_map(|d| Some(Hours { weekday: engine::weekday(d)?, from: 0, to: 1440 })).collect();
    Ok(Some(engine::calendar_spans(st.zone, &all_day, &[], &dated, window)))
}

/// A staff member's working time near `window` when calendars decide it (theirs, the business's):
/// their calendar's open time, or their weekly hours, inside the business's open time. `None`: their
/// weekly hours alone, exactly as without calendars.
pub async fn staff_open(tx: &mut Tx, tenant: Uuid, st: &Settings, s: &StaffInfo, business: &Option<Vec<Span>>, window: Span) -> Result<Option<Vec<Span>>> {
    let own = match &s.calendar {
        Some(name) => Some(calendar_open(tx, tenant, st, name, false, window).await?),
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

type BookedRow = (Uuid, Uuid, Uuid, Option<Uuid>, Instant, Instant, Instant, Instant, String, Option<i64>);

/// The engine's view of `svc` for `staff` around `around`: their time off and every active
/// booking of theirs or on the service's resources (two days of margin, so buffers and whole local
/// days are covered). `exclude` leaves out the booking being moved.
pub async fn world(tx: &mut Tx, tenant: Uuid, svc: &ServiceInfo, staff: &[StaffInfo], st: &Settings, around: Span, exclude: Option<Uuid>) -> Result<World> {
    let from = around.start - Duration::days(2);
    let to = around.end + Duration::days(2);
    let staff_ids: Vec<Uuid> = staff.iter().map(|s| s.id).collect();
    let range = Span::new(from, to);
    let resources: Vec<(Uuid, Option<i64>, Option<String>)> = if svc.resources.is_empty() {
        vec![]
    } else {
        sqlx::query_as("select id, capacity, calendar from e_booking_resource where tenant_id = $1 and id = any($2) and active")
            .bind(tenant)
            .bind(&svc.resources)
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
    let business = business_open(tx, tenant, st, range).await?;
    let mut working = Vec::new();
    for s in staff.iter().filter(|s| s.active) {
        working.push(engine::Staff {
            id: s.id,
            tz: s.tz,
            hours: s.hours.clone(),
            time_off: off.iter().filter(|o| o.0 == s.id).map(|o| Span::new(o.1, o.2)).collect(),
            busy: elsewhere.iter().filter(|o| o.0 == s.id).map(|o| Span::new(o.1, o.2)).collect(),
            open: staff_open(tx, tenant, st, s, &business, range).await?,
        });
    }
    // In the service's order of preference.
    let mut usable = Vec::new();
    for (id, capacity, cal) in svc.resources.iter().filter_map(|r| resources.iter().find(|x| x.0 == *r)) {
        let open = match cal.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(name) => Some(calendar_open(tx, tenant, st, name, false, range).await?),
            None => None,
        };
        usable.push(engine::Resource { id: *id, capacity: capacity.unwrap_or(1).max(1), open });
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
    })
}
