//! The availability engine: pure functions from working hours, time off, existing bookings and
//! resources to bookable start times. No I/O, so every rule — time zones and DST, the slot grid,
//! buffers, class seats, resource capacity, notice and advance windows, the fair choice of staff —
//! is unit-tested here, and the same code decides both the slots offered and the bookings taken.
//!
//! Semantics, in one place:
//! * Working hours are local wall-clock intervals in the staff member's IANA zone. Intervals that
//!   touch (a night shift split at midnight) merge. A local time skipped by a DST jump means the
//!   first instant after the gap; a repeated one means its first occurrence (its second for ends).
//! * The appointment `[start, start + duration)` must lie inside working time and outside time off.
//! * Each booking blocks `[start - buffer before, end + buffer after)` of its staff member; blocks of
//!   different sessions may not overlap (the database enforces the same rule with an exclusion
//!   constraint). Seats of one class session share their block.
//! * A resource hosts at most `capacity` sessions at any instant; joining a class session uses the
//!   session's resource and takes no extra unit.
//! * Offered starts lie on a grid every `interval` from the start of each working interval; class
//!   sessions that already exist are offered at their own start.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

pub type Instant = DateTime<Utc>;

/// A half-open time range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: Instant,
    pub end: Instant,
}

impl Span {
    pub fn new(start: Instant, end: Instant) -> Span {
        Span { start, end }
    }
    pub fn overlaps(&self, o: &Span) -> bool {
        self.start < o.end && o.start < self.end
    }
    pub fn contains(&self, o: &Span) -> bool {
        self.start <= o.start && o.end <= self.end
    }
    pub fn holds(&self, t: Instant) -> bool {
        self.start <= t && t < self.end
    }
    pub fn minutes(&self) -> i64 {
        (self.end - self.start).num_minutes().max(0)
    }
    /// The part of `self` inside `o` (empty, `start == end`, when they do not overlap).
    pub fn clip(&self, o: &Span) -> Span {
        let start = self.start.max(o.start);
        Span { start, end: self.end.min(o.end).max(start) }
    }
}

/// Sorted, non-overlapping spans; touching spans merge.
pub fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.end > s.start);
    spans.sort();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if s.start <= last.end => last.end = last.end.max(s.end),
            _ => out.push(s),
        }
    }
    out
}

/// `spans` minus `holes` (both merged first).
pub fn subtract(spans: Vec<Span>, holes: Vec<Span>) -> Vec<Span> {
    let holes = merge(holes);
    let mut out = Vec::new();
    for s in merge(spans) {
        let mut cur = s.start;
        for h in holes.iter().filter(|h| h.overlaps(&s)) {
            if h.start > cur {
                out.push(Span::new(cur, h.start));
            }
            cur = cur.max(h.end);
        }
        if cur < s.end {
            out.push(Span::new(cur, s.end));
        }
    }
    out
}

/// One working interval: minutes after local midnight, `[from, to)`, with `to` ≤ 1440 (24:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hours {
    pub weekday: Weekday,
    pub from: u32,
    pub to: u32,
}

/// `HH:MM` (24-hour, `24:00` allowed) as minutes after midnight.
pub fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (m < 60 && (h < 24 || (h == 24 && m == 0))).then_some(h * 60 + m)
}

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

pub fn weekday(name: &str) -> Option<Weekday> {
    WEEKDAYS.iter().position(|d| *d == name).and_then(|i| Weekday::try_from(i as u8).ok())
}

/// The instant a local wall-clock time names in `tz` (`minute` after `date`'s midnight; 1440 is
/// the next midnight). Skipped times resolve to the first instant after the gap; repeated ones to
/// their first occurrence, or their second with `late`.
pub fn local_instant(tz: Tz, date: NaiveDate, minute: u32, late: bool) -> Instant {
    let base = date.and_hms_opt(0, 0, 0).unwrap_or_default() + Duration::minutes(minute as i64);
    // Gaps last an hour or less in practice; whole skipped days (Samoa, 2011) are covered too.
    for shift in 0..=2880 {
        match tz.from_local_datetime(&(base + Duration::minutes(shift))) {
            LocalResult::Single(t) => return t.with_timezone(&Utc),
            LocalResult::Ambiguous(first, second) => {
                let chosen = if late && shift == 0 { second } else { first };
                return chosen.with_timezone(&Utc);
            }
            LocalResult::None => continue,
        }
    }
    base.and_utc()
}

/// The instants of one local calendar day in `tz`.
pub fn local_day(tz: Tz, date: NaiveDate) -> Span {
    Span::new(local_instant(tz, date, 0, false), local_instant(tz, date, 1440, false))
}

/// The local days `from..=to` in `tz` as one range of instants.
pub fn local_days(tz: Tz, from: NaiveDate, to: NaiveDate) -> Span {
    Span::new(local_day(tz, from).start, local_day(tz, to).end)
}

/// Working time in `tz` for every local day touching `window` (one day of margin on each side,
/// so shifts crossing midnight stay whole), merged.
pub fn working_spans(tz: Tz, hours: &[Hours], window: Span) -> Vec<Span> {
    let mut day = window.start.with_timezone(&tz).date_naive() - Duration::days(1);
    let last = window.end.with_timezone(&tz).date_naive() + Duration::days(1);
    let mut spans = Vec::new();
    while day <= last {
        for h in hours.iter().filter(|h| h.weekday == day.weekday() && h.to > h.from) {
            spans.push(Span::new(local_instant(tz, day, h.from, false), local_instant(tz, day, h.to, true)));
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    merge(spans)
}

/// Minutes of working time inside `period`, less time off.
pub fn available_minutes(tz: Tz, hours: &[Hours], time_off: &[Span], period: Span) -> i64 {
    let work: Vec<Span> = working_spans(tz, hours, period).iter().map(|s| s.clip(&period)).collect();
    subtract(work, time_off.to_vec()).iter().map(Span::minutes).sum()
}

#[derive(Debug, Clone)]
pub struct Staff {
    pub id: Uuid,
    pub tz: Tz,
    pub hours: Vec<Hours>,
    pub time_off: Vec<Span>,
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub id: Uuid,
    pub capacity: i64,
}

/// An active booking that holds time (and maybe a resource).
#[derive(Debug, Clone)]
pub struct Booked {
    pub id: Uuid,
    pub staff: Uuid,
    pub service: Uuid,
    pub resource: Option<Uuid>,
    /// The appointment itself.
    pub span: Span,
    /// The appointment with its buffers.
    pub block: Span,
    pub session: String,
    pub seats: i64,
}

#[derive(Debug, Clone)]
pub struct Service {
    pub id: Uuid,
    pub duration: Duration,
    pub before: Duration,
    pub after: Duration,
    /// Seats per session: 1 = one-to-one.
    pub capacity: i64,
    pub interval: Duration,
}

/// Limits on starts: notice and advance windows (public bookings), and whether a start must be a
/// grid point (guests pick offered slots; staff may book any time inside working hours).
#[derive(Debug, Clone, Copy, Default)]
pub struct Rules {
    pub earliest: Option<Instant>,
    pub latest: Option<Instant>,
    pub on_grid: bool,
}

/// Why a staff member cannot take a booking at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    TooManySeats,
    TooSoon,
    TooLate,
    OutsideHours,
    OffGrid,
    TimeOff,
    Busy,
    Full,
    NoResource,
}

impl Why {
    pub fn message(self) -> &'static str {
        match self {
            Why::TooManySeats => "more seats than the service has",
            Why::TooSoon => "too soon: it is inside the minimum notice",
            Why::TooLate => "too far ahead",
            Why::OutsideHours => "outside working hours",
            Why::OffGrid => "not one of the offered start times",
            Why::TimeOff => "on time off",
            Why::Busy => "already booked then",
            Why::Full => "the class is full",
            Why::NoResource => "no room or equipment is free then",
        }
    }
}

/// One way to take a booking: who, with which resource, in which session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub staff: Uuid,
    pub resource: Option<Uuid>,
    /// The class session joined or opened; `None` for one-to-one bookings (their own id).
    pub session: Option<String>,
    pub join: bool,
    /// Free seats in the session before this booking.
    pub seats_left: i64,
    /// Sessions and booked minutes the staff member has that local day (fairness).
    pub day_sessions: usize,
    pub day_minutes: i64,
}

/// A bookable start with every staff member who can take it.
#[derive(Debug, Clone)]
pub struct Slot {
    pub start: Instant,
    pub end: Instant,
    pub options: Vec<Candidate>,
}

/// Everything the engine needs about one service: its eligible staff (with their calendars),
/// eligible resources (empty = none needed) and the active bookings around the time in question.
#[derive(Debug, Clone)]
pub struct World {
    pub service: Service,
    pub staff: Vec<Staff>,
    pub resources: Vec<Resource>,
    pub booked: Vec<Booked>,
    pub rules: Rules,
}

/// The fair choice: join an existing class session first (classes fill up), otherwise the staff
/// member with the fewest sessions that day, then the fewest booked minutes, then a stable order.
pub fn pick(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates.iter().min_by_key(|c| (!c.join, c.day_sessions, c.day_minutes, c.staff))
}

impl World {
    pub fn group(&self) -> bool {
        self.service.capacity > 1
    }

    /// The key every seat of one class session shares (the exclusion constraint lets equal keys
    /// overlap).
    pub fn session_key(&self, staff: Uuid, start: Instant) -> String {
        format!("{}/{}/{}", self.service.id.simple(), staff.simple(), start.timestamp())
    }

    fn mine(&self, staff: Uuid) -> Vec<&Booked> {
        self.booked.iter().filter(|b| b.staff == staff).collect()
    }

    /// Whether `staff` (with working time `spans` and bookings `mine`) can take `seats` at `start`.
    fn check(&self, staff: &Staff, spans: &[Span], mine: &[&Booked], start: Instant, seats: i64) -> Result<Candidate, Why> {
        let svc = &self.service;
        if seats < 1 || seats > svc.capacity {
            return Err(Why::TooManySeats);
        }
        if self.rules.earliest.is_some_and(|e| start < e) {
            return Err(Why::TooSoon);
        }
        if self.rules.latest.is_some_and(|l| start > l) {
            return Err(Why::TooLate);
        }
        let appt = Span::new(start, start + svc.duration);
        let Some(work) = spans.iter().find(|w| w.contains(&appt)) else { return Err(Why::OutsideHours) };
        let key = self.group().then(|| self.session_key(staff.id, start));
        let session: Vec<&Booked> = match &key {
            Some(k) => mine.iter().copied().filter(|b| &b.session == k).collect(),
            None => vec![],
        };
        let join = !session.is_empty();
        let step = svc.interval.num_seconds().max(60);
        if self.rules.on_grid && !join && (start - work.start).num_seconds() % step != 0 {
            return Err(Why::OffGrid);
        }
        if staff.time_off.iter().any(|t| t.overlaps(&appt)) {
            return Err(Why::TimeOff);
        }
        let block = Span::new(start - svc.before, appt.end + svc.after);
        if mine.iter().any(|b| b.block.overlaps(&block) && key.as_ref() != Some(&b.session)) {
            return Err(Why::Busy);
        }
        let taken: i64 = session.iter().map(|b| b.seats).sum();
        if taken + seats > svc.capacity {
            return Err(Why::Full);
        }
        let resource = if join {
            session.iter().find_map(|b| b.resource)
        } else if self.resources.is_empty() {
            None
        } else {
            Some(self.free_resource(block).ok_or(Why::NoResource)?)
        };
        let day = local_day(staff.tz, start.with_timezone(&staff.tz).date_naive());
        let mut day_load: HashMap<&str, i64> = HashMap::new();
        for b in mine.iter().filter(|b| day.holds(b.span.start)) {
            day_load.insert(&b.session, b.span.minutes());
        }
        Ok(Candidate {
            staff: staff.id,
            resource,
            session: key,
            join,
            seats_left: svc.capacity - taken,
            day_sessions: day_load.len(),
            day_minutes: day_load.values().sum(),
        })
    }

    /// Sessions using `resource` at the busiest instant of `window`.
    pub fn resource_load(&self, resource: Uuid, window: Span) -> i64 {
        let mut sessions: HashMap<&str, Span> = HashMap::new();
        for b in self.booked.iter().filter(|b| b.resource == Some(resource) && b.block.overlaps(&window)) {
            sessions
                .entry(&b.session)
                .and_modify(|s| {
                    s.start = s.start.min(b.block.start);
                    s.end = s.end.max(b.block.end);
                })
                .or_insert(b.block);
        }
        let mut events: Vec<(Instant, i64)> = sessions
            .values()
            .flat_map(|s| {
                let c = s.clip(&window);
                [(c.start, 1), (c.end, -1)]
            })
            .collect();
        // Ends sort before starts at the same instant: touching ranges do not overlap.
        events.sort();
        let (mut now, mut peak) = (0, 0);
        for (_, d) in events {
            now += d;
            peak = peak.max(now);
        }
        peak
    }

    /// The least loaded eligible resource with a unit free throughout `block`.
    fn free_resource(&self, block: Span) -> Option<Uuid> {
        self.resources
            .iter()
            .map(|r| (self.resource_load(r.id, block), r))
            .filter(|(load, r)| *load < r.capacity)
            .min_by_key(|(load, r)| (*load, r.id))
            .map(|(_, r)| r.id)
    }

    /// Every staff member's answer for `seats` at `start`: the candidates, and why the others cannot.
    pub fn candidates_at(&self, start: Instant, seats: i64) -> (Vec<Candidate>, Vec<(Uuid, Why)>) {
        let around = Span::new(start - Duration::days(1), start + self.service.duration + Duration::days(1));
        let (mut ok, mut no) = (Vec::new(), Vec::new());
        for s in &self.staff {
            let spans = working_spans(s.tz, &s.hours, around);
            match self.check(s, &spans, &self.mine(s.id), start, seats) {
                Ok(c) => ok.push(c),
                Err(w) => no.push((s.id, w)),
            }
        }
        (ok, no)
    }

    /// Bookable starts inside `window` for `seats`, earliest first.
    pub fn slots(&self, window: Span, seats: i64) -> Vec<Slot> {
        let svc = &self.service;
        let step = if svc.interval < Duration::minutes(1) { Duration::minutes(1) } else { svc.interval };
        let mut out: BTreeMap<Instant, Vec<Candidate>> = BTreeMap::new();
        for s in &self.staff {
            let spans = working_spans(s.tz, &s.hours, window);
            let mine = self.mine(s.id);
            let mut starts = Vec::new();
            for w in &spans {
                let mut t = w.start;
                while t + svc.duration <= w.end {
                    if window.holds(t) {
                        starts.push(t);
                    }
                    t += step;
                }
            }
            if self.group() {
                starts.extend(
                    mine.iter()
                        .filter(|b| b.service == svc.id && window.holds(b.span.start) && b.session == self.session_key(s.id, b.span.start))
                        .map(|b| b.span.start),
                );
            }
            starts.sort();
            starts.dedup();
            for t in starts {
                if let Ok(c) = self.check(s, &spans, &mine, t, seats) {
                    out.entry(t).or_default().push(c);
                }
            }
        }
        out.into_iter().map(|(start, options)| Slot { start, end: start + svc.duration, options }).collect()
    }

    /// A class session at `start` that exists but has no room for `seats`: where a waitlist entry
    /// goes. `(staff, resource, session key)`.
    pub fn full_session(&self, start: Instant, seats: i64) -> Option<(Uuid, Option<Uuid>, String)> {
        if !self.group() {
            return None;
        }
        self.staff.iter().find_map(|s| {
            let key = self.session_key(s.id, start);
            let rows: Vec<&Booked> = self.booked.iter().filter(|b| b.session == key).collect();
            let taken: i64 = rows.iter().map(|b| b.seats).sum();
            (taken > 0 && taken + seats > self.service.capacity).then(|| (s.id, rows.iter().find_map(|b| b.resource), key))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Instant {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn every_day(from: &str, to: &str) -> Vec<Hours> {
        WEEKDAYS.iter().map(|w| Hours { weekday: weekday(w).unwrap(), from: parse_hhmm(from).unwrap(), to: parse_hhmm(to).unwrap() }).collect()
    }

    fn staff(n: u128, tz: Tz, hours: Vec<Hours>) -> Staff {
        Staff { id: uid(n), tz, hours, time_off: vec![] }
    }

    fn service(minutes: i64, before: i64, after: i64, capacity: i64, interval: i64) -> Service {
        Service {
            id: uid(900),
            duration: Duration::minutes(minutes),
            before: Duration::minutes(before),
            after: Duration::minutes(after),
            capacity,
            interval: Duration::minutes(interval),
        }
    }

    fn world(svc: Service, staff: Vec<Staff>) -> World {
        World { service: svc, staff, resources: vec![], booked: vec![], rules: Rules::default() }
    }

    /// A booking made the way the engine would make it.
    fn book(w: &mut World, staff: u128, start: &str, seats: i64, resource: Option<u128>) -> Uuid {
        let s = t(start);
        let svc = w.service.clone();
        let id = Uuid::now_v7();
        let session = if w.group() { w.session_key(uid(staff), s) } else { id.to_string() };
        w.booked.push(Booked {
            id,
            staff: uid(staff),
            service: svc.id,
            resource: resource.map(uid),
            span: Span::new(s, s + svc.duration),
            block: Span::new(s - svc.before, s + svc.duration + svc.after),
            session,
            seats,
        });
        id
    }

    fn starts(slots: &[Slot]) -> Vec<String> {
        slots.iter().map(|s| s.start.format("%Y-%m-%dT%H:%MZ").to_string()).collect()
    }

    fn why(w: &World, start: &str) -> Vec<Why> {
        w.candidates_at(t(start), 1).1.into_iter().map(|(_, w)| w).collect()
    }

    #[test]
    fn hours_and_weekdays_parse_strictly() {
        assert_eq!(parse_hhmm("09:30"), Some(570));
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("24:00"), Some(1440));
        for bad in ["24:01", "9:30", "09:60", "09-30", "0930", "ab:cd", "+1:00", " 9:3"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
        assert_eq!(weekday("Sunday"), Some(Weekday::Sun));
        assert_eq!(weekday("Monday"), Some(Weekday::Mon));
        assert_eq!(weekday("monday"), None);
    }

    #[test]
    fn spans_merge_and_subtract() {
        let s = |a: &str, b: &str| Span::new(t(a), t(b));
        let merged = merge(vec![s("2026-01-01T12:00:00Z", "2026-01-01T13:00:00Z"), s("2026-01-01T09:00:00Z", "2026-01-01T12:00:00Z")]);
        assert_eq!(merged, vec![s("2026-01-01T09:00:00Z", "2026-01-01T13:00:00Z")], "touching spans merge");
        let left = subtract(
            vec![s("2026-01-01T09:00:00Z", "2026-01-01T17:00:00Z")],
            vec![
                s("2026-01-01T12:00:00Z", "2026-01-01T13:00:00Z"),
                s("2026-01-01T12:30:00Z", "2026-01-01T14:00:00Z"),
                s("2026-01-01T16:30:00Z", "2026-01-02T00:00:00Z"),
            ],
        );
        assert_eq!(left, vec![s("2026-01-01T09:00:00Z", "2026-01-01T12:00:00Z"), s("2026-01-01T14:00:00Z", "2026-01-01T16:30:00Z")]);
        assert_eq!(s("2026-01-01T09:00:00Z", "2026-01-01T10:00:00Z").clip(&s("2026-01-01T11:00:00Z", "2026-01-01T12:00:00Z")).minutes(), 0);
    }

    #[test]
    fn local_times_across_dst_jumps() {
        let ny: Tz = "America/New_York".parse().unwrap();
        // 2026-03-08: 02:00 EST jumps to 03:00 EDT. 02:30 does not exist: the gap's end is used.
        assert_eq!(local_instant(ny, d(2026, 3, 8), 150, false), t("2026-03-08T07:00:00Z"));
        assert_eq!(local_instant(ny, d(2026, 3, 8), 60, false), t("2026-03-08T06:00:00Z"), "01:00 EST");
        assert_eq!(local_instant(ny, d(2026, 3, 8), 180, false), t("2026-03-08T07:00:00Z"), "03:00 EDT");
        // 2026-11-01: 02:00 EDT falls back to 01:00 EST. 01:30 happens twice.
        assert_eq!(local_instant(ny, d(2026, 11, 1), 90, false), t("2026-11-01T05:30:00Z"), "first 01:30 (EDT)");
        assert_eq!(local_instant(ny, d(2026, 11, 1), 90, true), t("2026-11-01T06:30:00Z"), "second 01:30 (EST)");
        // Local days are 23 and 25 hours long on those dates.
        assert_eq!(local_day(ny, d(2026, 3, 8)).minutes(), 23 * 60);
        assert_eq!(local_day(ny, d(2026, 11, 1)).minutes(), 25 * 60);
        assert_eq!(local_day(ny, d(2026, 11, 2)).minutes(), 24 * 60);
        // A zone that skipped a whole day (Samoa, 30 December 2011) still resolves.
        let apia: Tz = "Pacific/Apia".parse().unwrap();
        assert_eq!(local_instant(apia, d(2011, 12, 30), 600, false), t("2011-12-30T10:00:00Z"));
    }

    #[test]
    fn slots_follow_the_grid_inside_working_hours() {
        let utc: Tz = "UTC".parse().unwrap();
        let w = world(service(60, 0, 0, 1, 30), vec![staff(1, utc, every_day("09:00", "12:00"))]);
        let day = local_day(utc, d(2026, 6, 1));
        assert_eq!(starts(&w.slots(day, 1)), ["2026-06-01T09:00Z", "2026-06-01T09:30Z", "2026-06-01T10:00Z", "2026-06-01T10:30Z", "2026-06-01T11:00Z"]);
        // Breaks: two intervals a day restart the grid; a 45-minute service needs 45 free minutes.
        let mut split = every_day("09:00", "10:30");
        split.extend(every_day("13:15", "14:30"));
        let w = world(service(45, 0, 0, 1, 30), vec![staff(1, utc, split)]);
        assert_eq!(starts(&w.slots(day, 1)), ["2026-06-01T09:00Z", "2026-06-01T09:30Z", "2026-06-01T13:15Z", "2026-06-01T13:45Z"]);
    }

    #[test]
    fn staff_zones_and_dst_days_keep_local_working_hours() {
        // Working 09:00-11:00 in London. Before 29 March 2026 London is UTC+0, after it UTC+1.
        let london: Tz = "Europe/London".parse().unwrap();
        let w = world(service(60, 0, 0, 1, 60), vec![staff(1, london, every_day("09:00", "11:00"))]);
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(starts(&w.slots(local_day(utc, d(2026, 3, 28)), 1)), ["2026-03-28T09:00Z", "2026-03-28T10:00Z"]);
        assert_eq!(starts(&w.slots(local_day(utc, d(2026, 3, 29)), 1)), ["2026-03-29T08:00Z", "2026-03-29T09:00Z"], "the DST day itself");
        // A guest in Kolkata (UTC+05:30) asking for their 29 March sees those same instants.
        let kolkata: Tz = "Asia/Kolkata".parse().unwrap();
        let guest_day = local_day(kolkata, d(2026, 3, 29));
        assert_eq!(guest_day, Span::new(t("2026-03-28T18:30:00Z"), t("2026-03-29T18:30:00Z")));
        let seen = w.slots(guest_day, 1);
        assert_eq!(starts(&seen), ["2026-03-29T08:00Z", "2026-03-29T09:00Z"]);
        assert_eq!(seen[0].start.with_timezone(&kolkata).format("%H:%M").to_string(), "13:30");
        // A guest in Los Angeles asking for their 28 March sees London's 28 March morning only:
        // LA's day ends at 07:00 UTC on the 29th, before London opens.
        let la: Tz = "America/Los_Angeles".parse().unwrap();
        assert_eq!(starts(&w.slots(local_day(la, d(2026, 3, 28)), 1)), ["2026-03-28T09:00Z", "2026-03-28T10:00Z"]);
        // New York staff on the US fall-back night: 00:00-03:00 local spans four real hours.
        let ny: Tz = "America/New_York".parse().unwrap();
        let w = world(service(60, 0, 0, 1, 60), vec![staff(2, ny, every_day("00:00", "03:00"))]);
        assert_eq!(starts(&w.slots(local_day(ny, d(2026, 11, 1)), 1)), ["2026-11-01T04:00Z", "2026-11-01T05:00Z", "2026-11-01T06:00Z", "2026-11-01T07:00Z"]);
        // ...and on the spring-forward night only two: 02:00-03:00 does not exist.
        assert_eq!(starts(&w.slots(local_day(ny, d(2026, 3, 8)), 1)), ["2026-03-08T05:00Z", "2026-03-08T06:00Z"]);
    }

    #[test]
    fn night_shifts_merge_across_midnight() {
        let utc: Tz = "UTC".parse().unwrap();
        let hours = vec![
            Hours { weekday: Weekday::Mon, from: parse_hhmm("22:00").unwrap(), to: 1440 },
            Hours { weekday: Weekday::Tue, from: 0, to: parse_hhmm("02:00").unwrap() },
        ];
        let w = world(service(90, 0, 0, 1, 30), vec![staff(1, utc, hours)]);
        // 2026-06-01 is a Monday.
        let (ok, _) = w.candidates_at(t("2026-06-01T23:30:00Z"), 1);
        assert_eq!(ok.len(), 1, "an appointment may run past midnight inside one shift");
        assert_eq!(why(&w, "2026-06-02T01:00:00Z"), [Why::OutsideHours], "but not past the shift's end");
    }

    #[test]
    fn buffers_keep_bookings_apart() {
        let utc: Tz = "UTC".parse().unwrap();
        let mut w = world(service(60, 0, 15, 1, 30), vec![staff(1, utc, every_day("09:00", "13:00"))]);
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, None);
        // The 10:00 booking blocks until 11:15 with its after-buffer: 11:00 is taken, 11:30 is not.
        // 09:00 ends at 10:00 but its own buffer runs to 10:15, into the 10:00 booking.
        let day = local_day(utc, d(2026, 6, 1));
        assert_eq!(starts(&w.slots(day, 1)), ["2026-06-01T11:30Z", "2026-06-01T12:00Z"]);
        assert_eq!(why(&w, "2026-06-01T09:00:00Z"), [Why::Busy]);
        assert_eq!(why(&w, "2026-06-01T11:00:00Z"), [Why::Busy]);
        // A buffer may reach outside working hours: the last booking can end at closing time.
        assert!(w.slots(day, 1).iter().any(|s| s.end == t("2026-06-01T13:00:00Z")));
        // Before-buffers count too.
        let mut w = world(service(60, 30, 0, 1, 30), vec![staff(1, utc, every_day("09:00", "13:00"))]);
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, None);
        assert_eq!(why(&w, "2026-06-01T11:00:00Z"), [Why::Busy], "11:00 needs 10:30-11:00 free to prepare");
        assert!(w.candidates_at(t("2026-06-01T11:30:00Z"), 1).0.len() == 1);
    }

    #[test]
    fn time_off_notice_and_advance() {
        let utc: Tz = "UTC".parse().unwrap();
        let mut s = staff(1, utc, every_day("09:00", "17:00"));
        s.time_off = vec![Span::new(t("2026-06-01T12:00:00Z"), t("2026-06-01T14:00:00Z"))];
        let mut w = world(service(60, 0, 0, 1, 60), vec![s]);
        let day = local_day(utc, d(2026, 6, 1));
        assert_eq!(
            starts(&w.slots(day, 1)),
            ["2026-06-01T09:00Z", "2026-06-01T10:00Z", "2026-06-01T11:00Z", "2026-06-01T14:00Z", "2026-06-01T15:00Z", "2026-06-01T16:00Z"]
        );
        w.rules = Rules { earliest: Some(t("2026-06-01T10:30:00Z")), latest: Some(t("2026-06-01T15:00:00Z")), on_grid: true };
        assert_eq!(starts(&w.slots(day, 1)), ["2026-06-01T11:00Z", "2026-06-01T14:00Z", "2026-06-01T15:00Z"]);
        assert_eq!(why(&w, "2026-06-01T10:00:00Z"), [Why::TooSoon]);
        assert_eq!(why(&w, "2026-06-01T16:00:00Z"), [Why::TooLate]);
        assert_eq!(why(&w, "2026-06-01T12:00:00Z"), [Why::TimeOff]);
        assert_eq!(why(&w, "2026-06-01T10:40:00Z"), [Why::OffGrid], "guests pick offered starts");
        w.rules.on_grid = false;
        assert!(why(&w, "2026-06-01T10:40:00Z").is_empty(), "staff may book any minute inside working hours");
        assert_eq!(why(&w, "2026-06-01T16:30:00Z"), [Why::TooLate]);
        w.rules = Rules::default();
        assert_eq!(why(&w, "2026-06-01T16:30:00Z"), [Why::OutsideHours], "17:30 is after closing");
    }

    #[test]
    fn class_sessions_share_seats() {
        let utc: Tz = "UTC".parse().unwrap();
        let mut w = world(service(60, 0, 0, 3, 60), vec![staff(1, utc, every_day("09:00", "12:00"))]);
        book(&mut w, 1, "2026-06-01T10:00:00Z", 2, None);
        let (ok, _) = w.candidates_at(t("2026-06-01T10:00:00Z"), 1);
        assert_eq!((ok[0].join, ok[0].seats_left), (true, 1));
        assert_eq!(ok[0].session.as_deref(), Some(w.session_key(uid(1), t("2026-06-01T10:00:00Z")).as_str()));
        assert_eq!(why(&w, "2026-06-01T10:00:00Z"), Vec::<Why>::new());
        assert_eq!(w.candidates_at(t("2026-06-01T10:00:00Z"), 2).1, vec![(uid(1), Why::Full)], "two seats do not fit");
        assert_eq!(w.full_session(t("2026-06-01T10:00:00Z"), 2).map(|f| f.0), Some(uid(1)));
        assert_eq!(w.full_session(t("2026-06-01T10:00:00Z"), 1), None, "one seat still fits: no waitlist");
        assert_eq!(w.full_session(t("2026-06-01T11:00:00Z"), 1), None, "no session there");
        // Another session of the same staff may not overlap this one; guests see 10:30 off the grid.
        assert_eq!(why(&w, "2026-06-01T10:30:00Z"), [Why::Busy]);
        w.rules.on_grid = true;
        assert_eq!(why(&w, "2026-06-01T10:30:00Z"), [Why::OffGrid]);
        w.rules.on_grid = false;
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, None);
        assert_eq!(why(&w, "2026-06-01T10:00:00Z"), [Why::Full]);
        // Seats beyond the class size are refused outright.
        assert_eq!(w.candidates_at(t("2026-06-01T09:00:00Z"), 4).1, vec![(uid(1), Why::TooManySeats)]);
        // Sessions already booked off the grid are offered at their own start.
        let mut w = world(service(60, 0, 0, 3, 60), vec![staff(1, utc, every_day("09:00", "12:00"))]);
        w.rules.on_grid = true;
        book(&mut w, 1, "2026-06-01T09:30:00Z", 1, None);
        assert_eq!(starts(&w.slots(local_day(utc, d(2026, 6, 1)), 1)), ["2026-06-01T09:30Z", "2026-06-01T11:00Z"]);
    }

    #[test]
    fn resources_limit_parallel_bookings() {
        let utc: Tz = "UTC".parse().unwrap();
        let mut w = world(service(60, 0, 0, 1, 60), vec![staff(1, utc, every_day("09:00", "12:00")), staff(2, utc, every_day("09:00", "12:00"))]);
        w.resources = vec![Resource { id: uid(50), capacity: 1 }];
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, Some(50));
        assert_eq!(w.candidates_at(t("2026-06-01T10:00:00Z"), 1).1, vec![(uid(1), Why::Busy), (uid(2), Why::NoResource)], "one room, already used");
        let free = w.candidates_at(t("2026-06-01T11:00:00Z"), 1).0;
        assert_eq!(free.iter().map(|c| c.resource).collect::<Vec<_>>(), vec![Some(uid(50)), Some(uid(50))]);
        // A second room (or a room for two) lets staff 2 work in parallel; the least used room is chosen.
        w.resources.push(Resource { id: uid(51), capacity: 1 });
        let (ok, _) = w.candidates_at(t("2026-06-01T10:00:00Z"), 1);
        assert_eq!((ok.len(), ok[0].staff, ok[0].resource), (1, uid(2), Some(uid(51))));
        w.resources = vec![Resource { id: uid(52), capacity: 2 }];
        w.booked[0].resource = Some(uid(52));
        assert_eq!(w.resource_load(uid(52), Span::new(t("2026-06-01T09:00:00Z"), t("2026-06-01T12:00:00Z"))), 1);
        assert_eq!(w.candidates_at(t("2026-06-01T10:00:00Z"), 1).0[0].resource, Some(uid(52)));
        // Peak load, not the count of overlapping bookings: 09-10 and 10-11 never coincide.
        book(&mut w, 2, "2026-06-01T09:00:00Z", 1, Some(52));
        assert_eq!(w.resource_load(uid(52), Span::new(t("2026-06-01T09:00:00Z"), t("2026-06-01T11:00:00Z"))), 1);
        // Joining a class uses the session's resource without taking another unit.
        let mut w = world(service(60, 0, 0, 5, 60), vec![staff(1, utc, every_day("09:00", "12:00"))]);
        w.resources = vec![Resource { id: uid(60), capacity: 1 }];
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, Some(60));
        let (ok, _) = w.candidates_at(t("2026-06-01T10:00:00Z"), 2);
        assert_eq!((ok[0].join, ok[0].resource, ok[0].seats_left), (true, Some(uid(60)), 4));
    }

    #[test]
    fn any_staff_goes_to_the_least_booked_that_day() {
        let utc: Tz = "UTC".parse().unwrap();
        let mut w = world(
            service(30, 0, 0, 1, 30),
            vec![staff(1, utc, every_day("09:00", "17:00")), staff(2, utc, every_day("09:00", "17:00")), staff(3, utc, every_day("09:00", "17:00"))],
        );
        book(&mut w, 1, "2026-06-01T09:00:00Z", 1, None);
        book(&mut w, 1, "2026-06-01T09:30:00Z", 1, None);
        book(&mut w, 2, "2026-06-01T09:00:00Z", 1, None);
        book(&mut w, 3, "2026-06-02T09:00:00Z", 1, None); // another day: does not count
        book(&mut w, 3, "2026-06-01T15:00:00Z", 1, None);
        let (ok, _) = w.candidates_at(t("2026-06-01T12:00:00Z"), 1);
        assert_eq!(ok.len(), 3);
        let loads: Vec<(Uuid, usize)> = ok.iter().map(|c| (c.staff, c.day_sessions)).collect();
        assert_eq!(loads, vec![(uid(1), 2), (uid(2), 1), (uid(3), 1)]);
        // 2 and 3 tie on sessions and minutes: the stable order decides.
        assert_eq!(pick(&ok).map(|c| c.staff), Some(uid(2)));
        book(&mut w, 2, "2026-06-01T16:00:00Z", 1, None);
        let (ok, _) = w.candidates_at(t("2026-06-01T12:00:00Z"), 1);
        assert_eq!(pick(&ok).map(|c| c.staff), Some(uid(3)));
        // Classes fill before a new session opens, even with a busier teacher.
        let mut w = world(service(60, 0, 0, 4, 60), vec![staff(1, utc, every_day("09:00", "17:00")), staff(2, utc, every_day("09:00", "17:00"))]);
        book(&mut w, 1, "2026-06-01T10:00:00Z", 1, None);
        book(&mut w, 1, "2026-06-01T12:00:00Z", 1, None);
        let (ok, _) = w.candidates_at(t("2026-06-01T10:00:00Z"), 1);
        assert_eq!(pick(&ok).map(|c| (c.staff, c.join)), Some((uid(1), true)));
        // The staff member's local day counts, not UTC's: 23:30 UTC is already tomorrow in Kolkata.
        let kolkata: Tz = "Asia/Kolkata".parse().unwrap();
        let mut w = world(service(30, 0, 0, 1, 30), vec![staff(1, kolkata, every_day("00:00", "24:00"))]);
        book(&mut w, 1, "2026-06-01T23:30:00Z", 1, None);
        assert_eq!(w.candidates_at(t("2026-06-01T10:00:00Z"), 1).0[0].day_sessions, 0);
        assert_eq!(w.candidates_at(t("2026-06-02T01:00:00Z"), 1).0[0].day_sessions, 1);
    }

    #[test]
    fn utilization_counts_working_minutes_less_time_off() {
        let ny: Tz = "America/New_York".parse().unwrap();
        let hours = every_day("09:00", "17:00");
        let week = local_days(ny, d(2026, 10, 26), d(2026, 11, 1));
        assert_eq!(available_minutes(ny, &hours, &[], week), 7 * 8 * 60, "the fall-back night does not change a 09-17 day");
        let off = [Span::new(local_instant(ny, d(2026, 10, 27), 12 * 60, false), local_instant(ny, d(2026, 10, 28), 10 * 60, false))];
        assert_eq!(available_minutes(ny, &hours, &off, week), 7 * 8 * 60 - (5 * 60 + 60));
        let half = Span::new(local_instant(ny, d(2026, 10, 26), 13 * 60, false), week.end);
        assert_eq!(available_minutes(ny, &hours, &[], half), 4 * 60 + 6 * 8 * 60);
    }
}
