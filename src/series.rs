//! Repeating bookings: one booking made into a series, at the same local time each time.
//!
//! Each occurrence is an ordinary booking. The engine places it and checks it like any other, so
//! buffers, capacity, working hours and time off all hold, and each can be moved or cancelled on its
//! own afterwards. They share `series`. Times are kept in the staff member's zone, whose hours they
//! fall in, so 10:00 stays 10:00 across a change of the clocks.
//!
//! A series is made whole or not at all, unless the caller asks to skip what does not fit: a gap
//! nobody chose is worse than an answer that says which date is taken.

use crate::booking::{self, ACTIVE};
use crate::calendar;
use crate::engine::local_instant;
use crate::{Instant, doc_id, obj, time, ts};
use chrono::{Duration, Months, NaiveDate, Timelike};
use chrono_tz::Tz;
use ferrum_domain::util::{elevated, id, s, sys};
use ferrum_kernel::{Ctx, Doc, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// The most one series holds: a year of weeks.
const MAX_OCCURRENCES: u32 = 52;

/// How far apart occurrences are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Every {
    Days(u32),
    Months(u32),
}

impl Every {
    fn parse(v: Option<&str>) -> Result<Every> {
        match v {
            Some("week") => Ok(Every::Days(7)),
            Some("2 weeks") => Ok(Every::Days(14)),
            Some("4 weeks") => Ok(Every::Days(28)),
            Some("month") => Ok(Every::Months(1)),
            _ => Err(Error::field("every", "must be week, 2 weeks, 4 weeks or month")),
        }
    }
}

/// The local dates of a series, `first` included. Counted from `first` each time, so a series on the
/// 31st falls on the last day of shorter months and comes back to the 31st after them.
pub fn dates(first: NaiveDate, every: Every, count: u32) -> Vec<NaiveDate> {
    (0..count)
        .filter_map(|n| match every {
            Every::Days(days) => first.checked_add_signed(Duration::days((n * days).into())),
            Every::Months(months) => first.checked_add_months(Months::new(n * months)),
        })
        .collect()
}

/// `first`'s wall-clock time on `date` in `tz`.
pub fn same_local_time(first: Instant, tz: Tz, date: NaiveDate) -> Instant {
    let local = first.with_timezone(&tz);
    local_instant(tz, date, local.hour() * 60 + local.minute(), false)
}

/// The fields of a booking its occurrences carry over; the engine fills in the rest.
fn occurrence(booking: &Doc, series: Uuid, start: Instant) -> Doc {
    let mut next: Doc = ["service", "staff", "seats", "customer", "guest_name", "guest_email", "guest_phone", "guest_time_zone", "notes", "company"]
        .iter()
        .filter_map(|f| booking.get(*f).filter(|v| !v.is_null()).map(|v| (f.to_string(), v.clone())))
        .collect();
    next.insert("series".into(), json!(series));
    next.insert("start_time".into(), json!(ts(start)));
    next
}

/// Why a date cannot be had, as the engine said it.
fn refusal(e: &Error) -> String {
    match e {
        Error::Validation(fields) => fields.iter().map(|f| f.message.clone()).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

/// Makes `booking` the first of a series (`{every, count, skip_conflicts?}`) and books the rest in
/// its transaction.
pub async fn repeat(hc: &mut HookCtx<'_>, booking: &Doc, params: &Value) -> Result<Value> {
    repeat_in(hc.kernel, hc.ctx, hc.tx, booking, params).await
}

/// [`repeat`], for a caller with a transaction of its own: the booking page, for a guest.
pub async fn repeat_in(kernel: &Kernel, ctx: &Ctx, tx: &mut Tx, booking: &Doc, params: &Value) -> Result<Value> {
    if !ACTIVE.contains(&s(booking, "status").unwrap_or_default()) {
        return Err(Error::invalid("only a pending or confirmed booking can be repeated"));
    }
    if s(booking, "series").is_some() {
        return Err(Error::invalid("this booking is in a series already"));
    }
    let every = Every::parse(params.get("every").and_then(Value::as_str))?;
    let count = params.get("count").and_then(Value::as_u64).filter(|c| (2..=MAX_OCCURRENCES as u64).contains(c));
    let count = count.ok_or_else(|| Error::field("count", format!("must be 2 to {MAX_OCCURRENCES}, this booking included")))? as u32;
    // A page's dialog says "Yes"; the booking sheet sends true.
    let skip = params.get("skip_conflicts").is_some_and(|v| v.as_bool().unwrap_or(v.as_str() == Some("Yes")));
    let first = time(booking, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
    let staff = id(booking, "staff").ok_or_else(|| Error::internal("a placed booking has its staff"))?;
    let tz = calendar::staff(tx, ctx.tenant, &[staff]).await?.first().map(|st| st.tz).ok_or_else(|| Error::internal("a booking's staff is gone"))?;
    let series = Uuid::now_v7();
    let engine = elevated(ctx);
    let (mut booked, mut skipped) = (Vec::new(), Vec::new());
    for date in dates(first.with_timezone(&tz).date_naive(), every, count).into_iter().skip(1) {
        let start = same_local_time(first, tz, date);
        // A refused occurrence must not cost the others the transaction.
        sqlx::query("savepoint occurrence").execute(&mut **tx).await?;
        match kernel.insert_tx(&engine, tx, "Booking", occurrence(booking, series, start)).await {
            Ok(doc) => booked.push(json!({"id": doc.get("id"), "start_time": ts(start)})),
            Err(e @ (Error::Validation(_) | Error::Invalid(_) | Error::Conflict(_))) => {
                sqlx::query("rollback to savepoint occurrence").execute(&mut **tx).await?;
                let why = refusal(&e);
                if !skip {
                    return Err(Error::Invalid(format!(
                        "{date} cannot be booked: {why}. Nothing was repeated; choose another time, or skip the dates that do not fit"
                    )));
                }
                skipped.push(json!({"date": date, "start_time": ts(start), "reason": why}));
            }
            Err(e) => return Err(e),
        }
    }
    if booked.is_empty() {
        return Err(Error::invalid("none of the later dates can be booked"));
    }
    kernel.set_values_tx(&sys(ctx), tx, "Booking", doc_id(booking)?, obj(json!({"series": series}))).await?;
    Ok(json!({"series": series, "booked": booked, "skipped": skipped}))
}

/// Cancels this booking and the later ones of its series that are still active. Earlier ones, and
/// ones already over, stay.
pub async fn cancel_from(hc: &mut HookCtx<'_>, booking: &Doc, params: &Value) -> Result<Value> {
    let series = s(booking, "series").ok_or_else(|| Error::invalid("this booking is not in a series"))?;
    let from = time(booking, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
    let engine = elevated(hc.ctx);
    let filters = ferrum_kernel::query::parse_filters(&json!({"series": series}))?;
    let rows = hc.kernel.find_tx(&engine, hc.tx, "Booking", &filters, Some("start_time"), MAX_OCCURRENCES).await?;
    let reason = params.get("reason").and_then(Value::as_str);
    let mut cancelled = Vec::new();
    for row in rows {
        let due = time(&row, "start_time").is_some_and(|t| t >= from);
        if due && matches!(s(&row, "status"), Some("Pending" | "Confirmed" | "Waitlisted")) {
            booking::cancel(hc.kernel, &engine, hc.tx, &row, reason, false).await?;
            cancelled.push(row.get("id").cloned().unwrap_or(Value::Null));
        }
    }
    Ok(json!({"cancelled": cancelled}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn dates_keep_their_day() {
        assert_eq!(dates(day(2027, 3, 1), Every::Days(7), 3), vec![day(2027, 3, 1), day(2027, 3, 8), day(2027, 3, 15)]);
        assert_eq!(dates(day(2027, 3, 1), Every::Days(14), 2), vec![day(2027, 3, 1), day(2027, 3, 15)]);
        // The last day of shorter months, and back to the 31st after them.
        let months = dates(day(2027, 1, 31), Every::Months(1), 4);
        assert_eq!(months, vec![day(2027, 1, 31), day(2027, 2, 28), day(2027, 3, 31), day(2027, 4, 30)]);
        assert_eq!(dates(day(2028, 1, 31), Every::Months(1), 2)[1], day(2028, 2, 29), "a leap year");
        assert_eq!(dates(day(2027, 12, 15), Every::Months(1), 2)[1], day(2028, 1, 15), "over a new year");
    }

    #[test]
    fn local_time_stays_across_a_change_of_the_clocks() {
        let london: Tz = "Europe/London".parse().unwrap();
        let first = london.with_ymd_and_hms(2027, 10, 26, 10, 0, 0).unwrap().with_timezone(&chrono::Utc);
        assert_eq!(first.hour(), 9, "10:00 in summer time is 09:00 UTC");
        let next = same_local_time(first, london, day(2027, 11, 2));
        assert_eq!(next.hour(), 10, "and in winter time 10:00 UTC");
        assert_eq!(next.with_timezone(&london).hour(), 10, "the same 10:00 on the wall");
        // A time the clocks skip lands just after the gap, and one they repeat on its first pass.
        let early = london.with_ymd_and_hms(2027, 3, 21, 1, 30, 0).unwrap().with_timezone(&chrono::Utc);
        assert_eq!(same_local_time(early, london, day(2027, 3, 28)).with_timezone(&london).hour(), 2);
        let twice = london.with_ymd_and_hms(2027, 10, 24, 1, 30, 0).unwrap().with_timezone(&chrono::Utc);
        assert_eq!(same_local_time(twice, london, day(2027, 10, 31)).with_timezone(&london).minute(), 30);
    }

    #[test]
    fn only_what_is_offered_is_read() {
        assert_eq!(Every::parse(Some("week")).unwrap(), Every::Days(7));
        assert_eq!(Every::parse(Some("month")).unwrap(), Every::Months(1));
        assert!(Every::parse(Some("fortnightly")).is_err());
        assert!(Every::parse(None).is_err());
    }
}
