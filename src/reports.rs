//! Booking reports: volume by service / staff / status / source / day / month, no-show and
//! cancellation rates, and staff utilization (booked against available minutes). Periods are
//! workspace-local days; every query carries the caller's row scope on bookings, so staff with
//! own-bookings access see their own numbers.

use crate::calendar;
use crate::engine::{self, Span};
use async_trait::async_trait;
use chrono::NaiveDate;
use chrono_tz::Tz;
use ferrum_domain::reports::{Params, ReportCompany, ReportSource, col, param_def, period, read_tx};
use ferrum_domain::util::money;
use ferrum_kernel::{Action, Ctx, Error, Kernel, Result, Tx, row_scope};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

pub struct BookingReports;

const SUMMARY_GROUPS: [&str; 6] = ["Service", "Staff", "Status", "Source", "Day", "Month"];
const ATTENDANCE_GROUPS: [&str; 3] = ["Service", "Staff", "Month"];
/// Statuses whose time was taken (the booking happened, or was due to).
const HELD: &str = "('Pending', 'Confirmed', 'Completed', 'No Show')";

fn group_param(options: &[&str], default: &str) -> Value {
    json!({"name": "group_by", "label": "Group By", "type": "select", "options": options, "default": default})
}

fn grouping(p: &Params<'_>, options: &[&'static str], default: &'static str) -> Result<&'static str> {
    let want = p.str("group_by").unwrap_or(default);
    options.iter().copied().find(|o| o.eq_ignore_ascii_case(want)).ok_or_else(|| Error::field("group_by", format!("use one of: {}", options.join(", "))))
}

fn zone(ctx: &Ctx) -> Tz {
    calendar::zone(&ctx.schema.timezone).unwrap_or(Tz::UTC)
}

/// The report period as workspace-local days (default: the fiscal year to date).
fn span(ctx: &Ctx, p: &Params<'_>) -> Result<(NaiveDate, NaiveDate, Span)> {
    let co = ReportCompany { id: Uuid::nil(), name: String::new(), currency: String::new(), fy_month: ctx.fy_start_month, precision: 2 };
    let (from, to) = period(p, &co)?;
    Ok((from, to, engine::local_days(zone(ctx), from, to)))
}

fn push_group(qb: &mut QueryBuilder<Postgres>, label: &str, tz: &str) {
    match label {
        "Service" => qb.push("coalesce(a.service_name, 'Unknown')"),
        "Staff" => qb.push("coalesce(a.staff_name, 'Unassigned')"),
        "Status" => qb.push("coalesce(a.status, 'Unknown')"),
        "Source" => qb.push("coalesce(a.source, 'Internal')"),
        "Day" => qb.push("to_char(a.start_time at time zone ").push_bind(tz.to_string()).push(", 'YYYY-MM-DD')"),
        _ => qb.push("to_char(a.start_time at time zone ").push_bind(tz.to_string()).push(", 'YYYY-MM')"),
    };
}

/// `from e_booking a where` the caller's visible bookings starting inside `window`.
fn visible(ctx: &Ctx, qb: &mut QueryBuilder<Postgres>, window: Span) -> Result<()> {
    let ent = ctx.schema.entity("Booking").ok_or_else(|| Error::NotFound("Booking".into()))?;
    let scope = row_scope(ctx, ent, Action::Read)?;
    qb.push(" from e_booking a where a.tenant_id = ").push_bind(ctx.tenant);
    scope.push_sql(qb, "a");
    qb.push(" and a.start_time >= ").push_bind(window.start).push(" and a.start_time < ").push_bind(window.end);
    Ok(())
}

fn pct(part: i64, whole: i64) -> Value {
    if whole == 0 { Value::Null } else { money((Decimal::from(part) * Decimal::ONE_HUNDRED / Decimal::from(whole)).round_dp(2)) }
}

fn meta(from: NaiveDate, to: NaiveDate, group: Option<&str>, tz: Tz) -> Value {
    json!({"from_date": from, "to_date": to, "group_by": group, "time_zone": tz.name()})
}

type SummaryRow = (String, i64, i64, i64, i64, i64, i64, i64, i64, Decimal, i64);

async fn summary(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let (from, to, window) = span(ctx, p)?;
    let label = grouping(p, &SUMMARY_GROUPS, "Service")?;
    let tz = zone(ctx);
    let mut qb = QueryBuilder::new("select ");
    push_group(&mut qb, label, tz.name());
    qb.push(format!(
        ", count(*), coalesce(sum(a.seats) filter (where a.status in {HELD}), 0)::bigint, count(*) filter (where a.status = 'Pending'), \
         count(*) filter (where a.status = 'Confirmed'), count(*) filter (where a.status = 'Completed'), count(*) filter (where a.status = 'No Show'), \
         count(*) filter (where a.status = 'Cancelled'), count(*) filter (where a.status = 'Waitlisted'), \
         coalesce(sum(a.amount) filter (where a.status in {HELD}), 0), count(*) filter (where a.sales_invoice is not null)"
    ));
    visible(ctx, &mut qb, window)?;
    qb.push(" group by 1 order by 1");
    let data: Vec<SummaryRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let keys = ["bookings", "seats", "pending", "confirmed", "completed", "no_show", "cancelled", "waitlisted"];
    let mut totals = [0i64; 8];
    let mut total_amount = Decimal::ZERO;
    let mut invoiced_total = 0i64;
    let rows: Vec<Value> = data
        .into_iter()
        .map(|(group, a, b, c, d, e, f, g, h, amount, invoiced)| {
            let mut row = json!({"group": group});
            for ((k, t), x) in keys.iter().zip(totals.iter_mut()).zip([a, b, c, d, e, f, g, h]) {
                row[*k] = json!(x);
                *t += x;
            }
            row["amount"] = money(amount);
            row["invoiced"] = json!(invoiced);
            total_amount += amount;
            invoiced_total += invoiced;
            row
        })
        .collect();
    let mut t: serde_json::Map<String, Value> = keys.iter().zip(totals).map(|(k, v)| (k.to_string(), json!(v))).collect();
    t.insert("amount".into(), money(total_amount));
    t.insert("invoiced".into(), json!(invoiced_total));
    Ok(json!({
        "columns": [col("group", label, "data"), col("bookings", "Bookings", "int"), col("seats", "Seats Taken", "int"), col("pending", "Pending", "int"),
            col("confirmed", "Confirmed", "int"), col("completed", "Completed", "int"), col("no_show", "No Show", "int"), col("cancelled", "Cancelled", "int"),
            col("waitlisted", "Waitlisted", "int"), col("amount", "Booked Amount", "currency"), col("invoiced", "Invoiced", "int")],
        "rows": rows,
        "totals": t,
        "meta": meta(from, to, Some(label), tz),
    }))
}

async fn attendance(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let (from, to, window) = span(ctx, p)?;
    let label = grouping(p, &ATTENDANCE_GROUPS, "Month")?;
    let tz = zone(ctx);
    let mut qb = QueryBuilder::new("select ");
    push_group(&mut qb, label, tz.name());
    qb.push(
        ", count(*) filter (where a.status <> 'Waitlisted'), count(*) filter (where a.status = 'Completed'), \
         count(*) filter (where a.status = 'No Show'), count(*) filter (where a.status = 'Cancelled')",
    );
    visible(ctx, &mut qb, window)?;
    qb.push(" group by 1 order by 1");
    let data: Vec<(String, i64, i64, i64, i64)> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let row = |group: Option<String>, (n, done, absent, cancelled): (i64, i64, i64, i64)| {
        let mut r = json!({"bookings": n, "completed": done, "no_show": absent, "cancelled": cancelled,
            "cancellation_rate": pct(cancelled, n), "no_show_rate": pct(absent, done + absent)});
        if let Some(g) = group {
            r["group"] = json!(g);
        }
        r
    };
    let mut sum = (0, 0, 0, 0);
    let rows: Vec<Value> = data
        .into_iter()
        .map(|(g, n, done, absent, cancelled)| {
            sum = (sum.0 + n, sum.1 + done, sum.2 + absent, sum.3 + cancelled);
            row(Some(g), (n, done, absent, cancelled))
        })
        .collect();
    Ok(json!({
        "columns": [col("group", label, "data"), col("bookings", "Bookings", "int"), col("completed", "Completed", "int"), col("no_show", "No Show", "int"),
            col("cancelled", "Cancelled", "int"), col("cancellation_rate", "Cancellation Rate %", "percent"), col("no_show_rate", "No-Show Rate % (of attended or missed)", "percent")],
        "rows": rows,
        "totals": row(None, sum),
        "meta": meta(from, to, Some(label), tz),
    }))
}

async fn utilization(ctx: &Ctx, p: &Params<'_>, tx: &mut Tx) -> Result<Value> {
    let (from, to, window) = span(ctx, p)?;
    let ent = ctx.schema.entity("Booking").ok_or_else(|| Error::NotFound("Booking".into()))?;
    let scope = row_scope(ctx, ent, Action::Read)?;
    // Staff with own-bookings access see themselves only.
    let ids: Vec<Uuid> = if scope.is_unrestricted() {
        sqlx::query_scalar("select id from e_booking_staff where tenant_id = $1 order by display_name, id").bind(ctx.tenant).fetch_all(&mut **tx).await?
    } else {
        sqlx::query_scalar("select id from e_booking_staff where tenant_id = $1 and lower(email) = lower($2) order by display_name, id")
            .bind(ctx.tenant)
            .bind(&ctx.email)
            .fetch_all(&mut **tx)
            .await?
    };
    let staff = calendar::staff(tx, ctx.tenant, &ids).await?;
    let off: Vec<(Uuid, crate::Instant, crate::Instant)> = sqlx::query_as(
        "select staff, from_time, to_time from e_staff_time_off where tenant_id = $1 and staff = any($2) and to_time > $3 and from_time < $4 \
         and from_time is not null and to_time is not null",
    )
    .bind(ctx.tenant)
    .bind(&ids)
    .bind(window.start)
    .bind(window.end)
    .fetch_all(&mut **tx)
    .await?;
    // One stretch of staff time per session: a class with eight guests is one hour.
    let mut qb = QueryBuilder::new("select a.staff, min(a.start_time), max(a.end_time) from e_booking a where a.tenant_id = ");
    qb.push_bind(ctx.tenant);
    scope.push_sql(&mut qb, "a");
    qb.push(format!(" and a.status in {HELD} and a.staff = any(")).push_bind(ids.clone()).push(")");
    qb.push(" and a.end_time > ").push_bind(window.start).push(" and a.start_time < ").push_bind(window.end);
    qb.push(" and a.start_time is not null and a.end_time is not null group by a.staff, a.session_key");
    let sessions: Vec<(Uuid, crate::Instant, crate::Instant)> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let mut booked: HashMap<Uuid, (i64, i64)> = HashMap::new();
    for (sid, start, end) in sessions {
        let e = booked.entry(sid).or_default();
        e.0 += Span::new(start, end).clip(&window).minutes();
        e.1 += 1;
    }
    let (mut all_available, mut all_booked, mut all_sessions) = (0i64, 0i64, 0i64);
    let rows: Vec<Value> = staff
        .iter()
        .map(|s| {
            let away: Vec<Span> = off.iter().filter(|o| o.0 == s.id).map(|o| Span::new(o.1, o.2)).collect();
            let available = engine::available_minutes(s.tz, &s.hours, &away, window);
            let (minutes, count) = booked.get(&s.id).copied().unwrap_or_default();
            all_available += available;
            all_booked += minutes;
            all_sessions += count;
            json!({"staff": s.name, "time_zone": s.tz.name(), "available_minutes": available, "booked_minutes": minutes,
                "free_minutes": (available - minutes).max(0), "sessions": count, "utilization": pct(minutes, available)})
        })
        .collect();
    Ok(json!({
        "columns": [col("staff", "Staff", "data"), col("time_zone", "Time Zone", "data"), col("available_minutes", "Available Minutes", "int"),
            col("booked_minutes", "Booked Minutes", "int"), col("free_minutes", "Free Minutes", "int"), col("sessions", "Bookings / Sessions", "int"),
            col("utilization", "Utilization %", "percent")],
        "rows": rows,
        "totals": {"available_minutes": all_available, "booked_minutes": all_booked, "free_minutes": (all_available - all_booked).max(0),
            "sessions": all_sessions, "utilization": pct(all_booked, all_available)},
        "meta": meta(from, to, None, zone(ctx)),
    }))
}

#[async_trait]
impl ReportSource for BookingReports {
    fn catalog(&self) -> Vec<Value> {
        let entry =
            |name: &str, label: &str, params: Vec<Value>| json!({"name": name, "label": label, "module": "bookings", "entity": "Booking", "params": params});
        let span = || vec![param_def("from_date"), param_def("to_date")];
        vec![
            entry("bookings_summary", "Bookings Summary", [span(), vec![group_param(&SUMMARY_GROUPS, "Service")]].concat()),
            entry("bookings_attendance", "No-Shows and Cancellations", [span(), vec![group_param(&ATTENDANCE_GROUPS, "Month")]].concat()),
            entry("bookings_staff_utilization", "Staff Utilization", span()),
        ]
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, params: &HashMap<String, String>) -> Result<Value> {
        let p = Params { raw: params };
        let mut tx = read_tx(k, ctx).await?;
        let out = match name {
            "bookings_summary" => summary(ctx, &p, &mut tx).await,
            "bookings_attendance" => attendance(ctx, &p, &mut tx).await,
            "bookings_staff_utilization" => utilization(ctx, &p, &mut tx).await,
            _ => Err(Error::NotFound(format!("report {name}"))),
        };
        tx.rollback().await?;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_and_groupings() {
        assert_eq!(pct(1, 3), json!("33.33"));
        assert_eq!(pct(0, 0), Value::Null);
        let raw: HashMap<String, String> = [("group_by".to_string(), "staff".to_string())].into();
        assert_eq!(grouping(&Params { raw: &raw }, &SUMMARY_GROUPS, "Service").unwrap(), "Staff");
        let bad: HashMap<String, String> = [("group_by".to_string(), "Colour".to_string())].into();
        assert!(grouping(&Params { raw: &bad }, &ATTENDANCE_GROUPS, "Month").is_err());
        assert_eq!(BookingReports.catalog().len(), 3);
    }
}
