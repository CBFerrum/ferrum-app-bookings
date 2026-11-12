//! Staff members' outside calendars (FERRUM-BOOKINGS-004), read by their iCalendar addresses:
//! Google Calendar's secret address in iCal format, Outlook's published calendar, an iCloud or
//! Nextcloud subscription link, Ferrum Comms' own feed. Every quarter of an hour the upkeep reads
//! each address through the kernel's vetted client (https only, never a private address, a time
//! and a size limit) and keeps only when its staff member is busy, never what an event is. A busy
//! time is not offered to guests, as a booking is not.
//!
//! Events marked free (`TRANSP:TRANSPARENT`, or Outlook's free) or cancelled are not busy.
//! Recurring events are expanded with their exceptions by the icalendar crate (rrule
//! underneath), and a moved occurrence counts where it went. All-day events and floating times
//! are read in the staff member's zone, as is a zone a calendar names that is not known (Outlook's
//! Windows names are mapped first). The design is Cal.com's ICS feed calendar (MIT; none of its
//! code), read ahead of time rather than while a guest waits.

use crate::Instant;
use crate::engine::{Span, merge};
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use ferrum_kernel::{Ctx, Kernel, Result};
use ferrum_meta::Schema;
use icalendar::{Calendar, CalendarDateTime, Component, DatePerhapsTime, EventStatus};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

/// How often an outside calendar is read.
const EVERY: Duration = Duration::minutes(15);
/// What one reading may weigh and take.
const MAX_BYTES: usize = 5 << 20;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Occurrences of one recurring event taken at most.
const MAX_OCCURRENCES: u16 = 1000;

/// Windows zone names Outlook writes in TZID, mapped to IANA names (as Ferrum Comms maps them).
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("UTC", "UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("Romance Standard Time", "Europe/Paris"),
    ("E. Europe Standard Time", "Europe/Bucharest"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
];

/// What one reading found: the busy times, and how many events could not be read.
#[derive(Debug, PartialEq)]
pub(crate) struct Busy {
    pub spans: Vec<Span>,
    pub unread: usize,
}

/// When the calendar in `text` is busy between `from` and `to`, merged and in order. `tz`, the
/// staff member's zone, places all-day events, floating times and zones that are not known.
pub(crate) fn busy(text: &str, tz: Tz, from: Instant, to: Instant) -> std::result::Result<Busy, String> {
    let cal: Calendar = zones(&icalendar::parser::unfold(text), tz).parse().map_err(|_| "it is not an iCalendar file".to_string())?;
    // A moved or cancelled occurrence of a series is an event of its own, with the series' UID
    // and the start it replaces.
    let replaced: HashSet<(String, Instant)> = cal.events().filter_map(|e| Some((e.get_uid()?.to_string(), instant(&e.get_recurrence_id()?, tz)?))).collect();
    let (mut spans, mut unread) = (Vec::new(), 0);
    for e in cal.calendar_events() {
        if free(&e) {
            continue;
        }
        let Some(start) = e.get_start() else {
            unread += 1;
            continue;
        };
        let Some(length) = length(&e, &start, tz) else { continue };
        let series = e.get_recurrence_id().is_none() && (e.property_value("RRULE").is_some() || e.multi_properties().contains_key("RDATE"));
        let starts: Vec<Instant> = if series {
            let Ok(set) = e.get_recurrence() else {
                unread += 1;
                continue;
            };
            // Wide by a day each side: floating times are placed in `tz` only afterwards.
            let lead = length.at_most() + Duration::days(1);
            let (after, before) = ((from - lead).with_timezone(&icalendar::Tz::UTC), (to + Duration::days(1)).with_timezone(&icalendar::Tz::UTC));
            set.after(after).before(before).all(MAX_OCCURRENCES).dates.into_iter().filter_map(|o| occurrence(o, &length, tz)).collect()
        } else {
            instant(&start, tz).into_iter().collect()
        };
        let uid = e.get_uid().unwrap_or_default().to_string();
        for s in starts {
            if series && replaced.contains(&(uid.clone(), s)) {
                continue;
            }
            let Some(span) = length.from(s, tz) else { continue };
            if span.end > from && span.start < to {
                spans.push(span);
            }
        }
    }
    Ok(Busy { spans: merge(spans), unread })
}

/// How long an event lasts: whole days (an all-day event) or a time.
enum Length {
    Days(i64),
    Time(Duration),
}

impl Length {
    fn at_most(&self) -> Duration {
        match self {
            Length::Days(n) => Duration::days(*n + 1),
            Length::Time(d) => *d,
        }
    }

    /// The span starting at `start`: whole days run from midnight to midnight where `tz` is.
    fn from(&self, start: Instant, tz: Tz) -> Option<Span> {
        match self {
            Length::Time(d) => Some(Span::new(start, start + *d)),
            Length::Days(n) => {
                let day = start.with_timezone(&tz).date_naive();
                Some(Span::new(local(tz, day.and_hms_opt(0, 0, 0)?)?, local(tz, (day + Duration::days(*n)).and_hms_opt(0, 0, 0)?)?))
            }
        }
    }
}

fn length(e: &icalendar::Event, start: &DatePerhapsTime, tz: Tz) -> Option<Length> {
    let stated = e.property_value("DURATION").and_then(duration);
    match (start, e.get_end()) {
        (DatePerhapsTime::Date(a), Some(DatePerhapsTime::Date(b))) => Some(Length::Days((b - *a).num_days().max(1))),
        // A day, unless it says otherwise (RFC 5545 3.6.1).
        (DatePerhapsTime::Date(_), _) => Some(Length::Days(stated.map_or(1, |d| d.num_days().max(1)))),
        (_, Some(end)) => Some(Length::Time(instant(&end, tz)? - instant(start, tz)?)).filter(|l| matches!(l, Length::Time(d) if *d > Duration::zero())),
        // A moment without a length takes no time.
        (_, None) => stated.filter(|d| *d > Duration::zero()).map(Length::Time),
    }
}

/// An occurrence's start: floating ones (the rrule crate's local zone) are wall-clock times in
/// `tz`, and an all-day one is its day there.
fn occurrence(o: DateTime<icalendar::Tz>, length: &Length, tz: Tz) -> Option<Instant> {
    match (length, o.timezone()) {
        (Length::Days(_), _) => local(tz, o.naive_local().date().and_hms_opt(0, 0, 0)?),
        (_, icalendar::Tz::Local(_)) => local(tz, o.naive_local()),
        _ => Some(o.with_timezone(&Utc)),
    }
}

fn instant(v: &DatePerhapsTime, tz: Tz) -> Option<Instant> {
    match v {
        DatePerhapsTime::Date(d) => local(tz, d.and_hms_opt(0, 0, 0)?),
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(t)) => Some(*t),
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(n)) => local(tz, *n),
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, tzid }) => local(tzid.parse().unwrap_or(tz), *date_time),
    }
}

/// A wall-clock time where `tz` is; one a daylight-saving jump skips is read an hour later.
fn local(tz: Tz, n: NaiveDateTime) -> Option<Instant> {
    tz.from_local_datetime(&n).earliest().or_else(|| tz.from_local_datetime(&(n + Duration::hours(1))).earliest()).map(|t| t.with_timezone(&Utc))
}

fn free(e: &icalendar::Event) -> bool {
    let says = |key: &str, word: &str| e.property_value(key).is_some_and(|v| v.trim().eq_ignore_ascii_case(word));
    e.get_status() == Some(EventStatus::Cancelled) || says("TRANSP", "TRANSPARENT") || says("X-MICROSOFT-CDO-BUSYSTATUS", "FREE")
}

/// `P1W`, `P2D`, `PT1H30M`, `P1DT12H` (RFC 5545 3.3.6); a negative one is no length.
fn duration(s: &str) -> Option<Duration> {
    let s = s.trim().trim_start_matches('+');
    let rest = s.strip_prefix('P')?;
    let (mut total, mut n, mut time) = (Duration::zero(), String::new(), false);
    for c in rest.chars() {
        match c {
            '0'..='9' => n.push(c),
            'T' => time = true,
            _ => {
                let v: i64 = std::mem::take(&mut n).parse().ok()?;
                total += match (c, time) {
                    ('W', false) => Duration::weeks(v),
                    ('D', false) => Duration::days(v),
                    ('H', true) => Duration::hours(v),
                    ('M', true) => Duration::minutes(v),
                    ('S', true) => Duration::seconds(v),
                    _ => return None,
                };
            }
        }
    }
    n.is_empty().then_some(total)
}

/// The calendar's text with every TZID parameter naming a zone chrono-tz knows: Outlook's Windows
/// names mapped, any other unknown name taken as the staff member's own zone.
fn zones(text: &str, tz: Tz) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        match line.to_ascii_uppercase().find(";TZID=") {
            Some(at) => {
                let rest = &line[at + 6..];
                let (name, len) = match rest.strip_prefix('"') {
                    Some(q) => q.find('"').map_or((q, rest.len()), |e| (&q[..e], e + 2)),
                    None => {
                        let e = rest.find([':', ';']).unwrap_or(rest.len());
                        (&rest[..e], e)
                    }
                };
                out.push_str(&line[..at + 6]);
                out.push_str(known_zone(name).unwrap_or(tz).name());
                out.push_str(&rest[len..]);
            }
            None => out.push_str(line),
        }
        out.push_str("\r\n");
    }
    out
}

fn known_zone(name: &str) -> Option<Tz> {
    let n = name.trim().trim_start_matches('/');
    n.parse().ok().or_else(|| WINDOWS_ZONES.iter().find(|(w, _)| w.eq_ignore_ascii_case(n)).and_then(|(_, iana)| iana.parse().ok()))
}

/// Reads the outside calendars of `tenant`'s staff whose turn has come, each in a transaction of
/// its own. One that cannot be read keeps what was read before and says why on the staff record;
/// one removed from a staff record takes its busy times with it. Answers how many were read.
pub(crate) async fn read_due(k: &Kernel, tenant: Uuid, schema: Arc<Schema>, now: Instant) -> Result<usize> {
    let ctx = Ctx::system(tenant, schema);
    let mut tx = k.db.tenant_tx(tenant).await?;
    sqlx::query(
        "delete from bookings_outside_busy b where b.tenant_id = $1 and not exists (select 1 from record_secret r where r.tenant_id = b.tenant_id \
         and r.entity = 'BookingStaff' and r.record_id = b.staff and r.field = 'outside_calendar')",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    let due: Vec<(Uuid, String)> = sqlx::query_as(
        "select s.id, s.time_zone from e_booking_staff s join record_secret r on r.tenant_id = s.tenant_id and r.entity = 'BookingStaff' \
         and r.record_id = s.id and r.field = 'outside_calendar' where s.tenant_id = $1 and coalesce(s.active, false) \
         and (s.outside_calendar_tried is null or s.outside_calendar_tried <= $2) order by s.id",
    )
    .bind(tenant)
    .bind(now - EVERY)
    .fetch_all(&mut *tx)
    .await?;
    let settings = crate::settings::load(&mut tx, tenant).await?;
    // As far ahead as any service may be booked.
    let ahead: Option<i64> =
        sqlx::query_scalar("select max(max_advance_days) from e_booking_service where tenant_id = $1").bind(tenant).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let ahead = settings.max_advance.max(Duration::days(ahead.unwrap_or(0)));
    let (from, to) = (now - Duration::days(1), now + ahead + Duration::days(1));
    let mut read = 0;
    for (staff, zone) in due {
        let tz: Tz = zone.parse().unwrap_or(Tz::UTC);
        let mut tx = k.db.tenant_tx(tenant).await?;
        let Some(url) = k.secret(&mut tx, tenant, "BookingStaff", staff, "outside_calendar").await? else { continue };
        let found = match fetch(k, &url).await {
            Ok(text) => busy(&text, tz, from, to),
            Err(e) => Err(e),
        };
        let mut v = ferrum_kernel::Doc::new();
        v.insert("outside_calendar_tried".into(), json!(crate::ts(now)));
        match found {
            Ok(b) => {
                sqlx::query("delete from bookings_outside_busy where tenant_id = $1 and staff = $2").bind(tenant).bind(staff).execute(&mut *tx).await?;
                let (starts, ends): (Vec<Instant>, Vec<Instant>) = b.spans.iter().map(|s| (s.start, s.end)).unzip();
                sqlx::query(
                    "insert into bookings_outside_busy (tenant_id, staff, from_time, to_time) select $1, $2, f, t from unnest($3::timestamptz[], $4::timestamptz[]) as x(f, t)",
                )
                .bind(tenant)
                .bind(staff)
                .bind(&starts)
                .bind(&ends)
                .execute(&mut *tx)
                .await?;
                clashes(k, &ctx, &mut tx, staff, now).await?;
                v.insert("outside_calendar_read".into(), json!(crate::ts(now)));
                let problem = (b.unread > 0).then(|| format!("{} event{} could not be read", b.unread, if b.unread == 1 { "" } else { "s" }));
                v.insert("outside_calendar_error".into(), json!(problem));
                read += 1;
            }
            Err(e) => {
                v.insert("outside_calendar_error".into(), json!(e));
            }
        }
        k.set_values_tx(&ctx, &mut tx, "BookingStaff", staff, v).await?;
        tx.commit().await?;
    }
    Ok(read)
}

/// Marks the staff member's coming bookings that their outside calendar is now busy over, and
/// clears those it no longer is (FEO-032). A booking kept despite a clash is not marked again at its
/// time; moved, it is checked again. The staff member is told of each one newly marked, and the
/// guest's page says it may need another time.
async fn clashes(k: &Kernel, ctx: &Ctx, tx: &mut ferrum_kernel::Tx, staff: Uuid, now: Instant) -> Result<()> {
    let coming: Vec<(Uuid, bool, Option<Instant>, Option<Instant>, bool)> = sqlx::query_as(
        "select b.id, coalesce(b.outside_clash, false), b.outside_clash_kept, b.scheduled_at, exists (select 1 from bookings_outside_busy o \
         where o.tenant_id = b.tenant_id and o.staff = b.staff and o.from_time < b.end_time and o.to_time > b.start_time) \
         from e_booking b where b.tenant_id = $1 and b.staff = $2 and b.status in ('Pending', 'Confirmed') and b.start_time > $3",
    )
    .bind(ctx.tenant)
    .bind(staff)
    .bind(now)
    .fetch_all(&mut **tx)
    .await?;
    for (booking, marked, kept, scheduled, busy) in coming {
        let kept_here = kept.is_some_and(|k| scheduled.is_none_or(|s| k >= s));
        let v = match (busy, marked) {
            (true, false) if !kept_here => json!({"outside_clash": true, "outside_clash_since": crate::ts(now)}),
            (false, true) => json!({"outside_clash": false, "outside_clash_since": null}),
            _ => continue,
        };
        k.set_values_tx(ctx, tx, "Booking", booking, v.as_object().cloned().unwrap_or_default()).await?;
        if busy {
            let b = k.get_tx(ctx, tx, "Booking", booking, false).await?;
            crate::notify::tell_staff(tx, ctx.tenant, &b, crate::notify::StaffNote::Clash).await?;
        }
    }
    Ok(())
}

/// The calendar's text. Errors never repeat the address: it is a secret that opens the calendar.
async fn fetch(k: &Kernel, url: &str) -> std::result::Result<String, String> {
    let (client, parsed) = k.vetted_client(url, k.settings.webhooks_allow_private, TIMEOUT, "Ferrum Bookings").await.map_err(|e| {
        let e = e.to_string();
        if e.contains("must use https") {
            "its address must start with https://".to_string()
        } else if e.contains("private address") {
            "its address is on a private network".to_string()
        } else if e.contains("is not a URL") || e.contains("has no host") {
            "its address is not a web address".to_string()
        } else {
            "it could not be reached".to_string()
        }
    })?;
    let mut res = client
        .get(parsed)
        .header("Accept", "text/calendar")
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "it did not answer in time".to_string() } else { "it could not be reached".to_string() })?;
    if !res.status().is_success() {
        return Err(format!("it answered {}", res.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| "it stopped answering".to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            return Err("it is larger than 5 MB".into());
        }
    }
    String::from_utf8(body).map_err(|_| "it is not text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Instant {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn cal(events: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n{events}END:VCALENDAR\r\n")
    }

    fn spans(found: &Busy) -> Vec<(String, String)> {
        found.spans.iter().map(|s| (s.start.to_rfc3339(), s.end.to_rfc3339())).collect()
    }

    const LONDON: Tz = chrono_tz::Europe::London;

    #[test]
    fn busy_times_only() {
        let text = cal(concat!(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261110T090000Z\r\nDTEND:20261110T100000Z\r\nSUMMARY:Dentist\r\nEND:VEVENT\r\n",
            // Free, cancelled and Outlook's free: not busy.
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20261110T110000Z\r\nDTEND:20261110T120000Z\r\nTRANSP:TRANSPARENT\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:c\r\nDTSTART:20261110T130000Z\r\nDTEND:20261110T140000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20261110T150000Z\r\nDTEND:20261110T160000Z\r\nX-MICROSOFT-CDO-BUSYSTATUS:FREE\r\nEND:VEVENT\r\n",
            // Overlapping ones merge; one outside the window is left out.
            "BEGIN:VEVENT\r\nUID:e\r\nDTSTART:20261110T093000Z\r\nDURATION:PT1H\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:f\r\nDTSTART:20270110T090000Z\r\nDTEND:20270110T100000Z\r\nEND:VEVENT\r\n",
        ));
        let found = busy(&text, LONDON, t("2026-11-01T00:00:00Z"), t("2026-12-31T00:00:00Z")).unwrap();
        assert_eq!(spans(&found), [("2026-11-10T09:00:00+00:00".into(), "2026-11-10T10:30:00+00:00".into())]);
        assert_eq!(found.unread, 0);
        assert!(busy("not a calendar", LONDON, t("2026-11-01T00:00:00Z"), t("2026-12-31T00:00:00Z")).is_err());
    }

    #[test]
    fn series_with_exceptions_and_moves() {
        // Mondays at 9 in Berlin for four weeks; the second one taken out, the third moved to 14:00.
        let text = cal(concat!(
            "BEGIN:VEVENT\r\nUID:s\r\nDTSTART;TZID=W. Europe Standard Time:20261102T090000\r\nDTEND;TZID=W. Europe Standard Time:20261102T100000\r\n",
            "RRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE;TZID=W. Europe Standard Time:20261109T090000\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261116T090000\r\nDTSTART;TZID=Europe/Berlin:20261116T140000\r\n",
            "DTEND;TZID=Europe/Berlin:20261116T150000\r\nEND:VEVENT\r\n",
        ));
        let found = busy(&text, LONDON, t("2026-11-01T00:00:00Z"), t("2026-12-31T00:00:00Z")).unwrap();
        assert_eq!(
            spans(&found),
            [
                ("2026-11-02T08:00:00+00:00".into(), "2026-11-02T09:00:00+00:00".into()),
                ("2026-11-16T13:00:00+00:00".into(), "2026-11-16T14:00:00+00:00".into()),
                ("2026-11-23T08:00:00+00:00".into(), "2026-11-23T09:00:00+00:00".into()),
            ]
        );
    }

    #[test]
    fn days_and_floating_times_are_where_the_staff_member_is() {
        let text = cal(concat!(
            // A two-day trip, and a floating 9:00 (no zone), both in London's summer time.
            "BEGIN:VEVENT\r\nUID:trip\r\nDTSTART;VALUE=DATE:20260706\r\nDTEND;VALUE=DATE:20260708\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:float\r\nDTSTART:20260710T090000\r\nDTEND:20260710T093000\r\nEND:VEVENT\r\n",
            // A zone nobody knows is the staff member's own; a weekly all-day event recurs by day.
            "BEGIN:VEVENT\r\nUID:odd\r\nDTSTART;TZID=Somewhere/Else:20260711T090000\r\nDTEND;TZID=Somewhere/Else:20260711T100000\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:fridays\r\nDTSTART;VALUE=DATE:20260717\r\nRRULE:FREQ=WEEKLY;COUNT=2\r\nEND:VEVENT\r\n",
        ));
        let found = busy(&text, LONDON, t("2026-07-01T00:00:00Z"), t("2026-08-01T00:00:00Z")).unwrap();
        assert_eq!(
            spans(&found),
            [
                ("2026-07-05T23:00:00+00:00".into(), "2026-07-07T23:00:00+00:00".into()),
                ("2026-07-10T08:00:00+00:00".into(), "2026-07-10T08:30:00+00:00".into()),
                ("2026-07-11T08:00:00+00:00".into(), "2026-07-11T09:00:00+00:00".into()),
                ("2026-07-16T23:00:00+00:00".into(), "2026-07-17T23:00:00+00:00".into()),
                ("2026-07-23T23:00:00+00:00".into(), "2026-07-24T23:00:00+00:00".into()),
            ]
        );
    }

    #[test]
    fn durations() {
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1DT12H"), Some(Duration::hours(36)));
        assert_eq!(duration("P2W"), Some(Duration::days(14)));
        assert_eq!(duration("-PT15M"), None);
        assert_eq!(duration("PT"), Some(Duration::zero()));
        assert_eq!(duration("P1H"), None, "hours belong after T");
    }
}
