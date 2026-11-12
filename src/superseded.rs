//! Booking Settings' own dated openings and closures (`calendar_dates`, Bookings 0.6) are superseded by the
//! exceptions of the platform's working calendars, which every other app and the calendar's own functions read.
//! Once per workspace the upkeep moves them: each row into the exceptions of the calendar it names (the business
//! calendar when it names none; a business calendar is created when the workspace had dates but no calendar),
//! never over an exception the calendar already has for that date. The rows stay, readable, and are not read again.

use crate::engine::parse_hhmm;
use crate::obj;
use ferrum_kernel::{Ctx, Kernel, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// The calendar made for a workspace that had business-wide dates and named no business calendar.
pub const CREATED: &str = "bookings_business";

type Row = (Option<String>, Option<chrono::NaiveDate>, Option<String>, Option<String>, Option<String>);

fn clock(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// One row as a calendar exception: `(date, exception)`; `None` for a row that says nothing usable.
fn exception(row: &Row) -> Option<(String, Value)> {
    let (_, date, from, to, change) = row;
    let hhmm = |v: &Option<String>, whole: u32| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map_or(Some(whole), parse_hhmm);
    let (from, to) = (hhmm(from, 0)?, hhmm(to, 1440)?);
    if from >= to {
        return None;
    }
    let open = change.as_deref() == Some("Open");
    let mut x = json!({"open": open});
    if (from, to) != (0, 1440) {
        x["hours"] = json!([[clock(from), clock(to)]]);
    }
    Some((date.as_ref()?.to_string(), x))
}

/// Moves the workspace's dated rows into its calendars, once. Returns the exceptions written.
pub(crate) async fn move_dates(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let done: Option<i32> = sqlx::query_scalar("select 1 from bookings_calendar_moved where tenant_id = $1").bind(ctx.tenant).fetch_optional(&mut *tx).await?;
    if done.is_some() {
        return Ok(0);
    }
    let settings: Option<(Uuid, Option<String>)> =
        sqlx::query_as("select id, calendar from e_booking_settings where tenant_id = $1 limit 1").bind(ctx.tenant).fetch_optional(&mut *tx).await?;
    let rows: Vec<Row> = match &settings {
        Some((id, _)) => sqlx::query_as(
            "select calendar, date, start_time, end_time, change from e_booking_settings__calendar_dates where tenant_id = $1 and parent_id = $2 order by idx",
        )
        .bind(ctx.tenant)
        .bind(id)
        .fetch_all(&mut *tx)
        .await?,
        None => vec![],
    };
    let business = settings.as_ref().and_then(|s| s.1.as_deref()).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    // Per calendar, the exceptions to add (one per date: the first row for a date wins).
    let mut plan: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut created = false;
    let mut seen = HashSet::new();
    for row in &rows {
        let named = row.0.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
        let Some((date, x)) = exception(row) else { continue };
        let target = match (named, &business) {
            (Some(n), _) => n,
            (None, Some(b)) => b.clone(),
            (None, None) => {
                // Without a business calendar the whole day was open: an extra opening changed nothing.
                if x["open"] == json!(true) {
                    continue;
                }
                created = true;
                CREATED.to_string()
            }
        };
        if seen.insert((target.clone(), date.clone())) {
            plan.entry(target).or_default().insert(date, x);
        }
    }
    let zone: Option<String> =
        sqlx::query_scalar("select settings->>'timezone' from tenant where id = $1").bind(ctx.tenant).fetch_optional(&mut *tx).await?.flatten();
    let mut written = 0;
    let mut puts = vec![];
    for (name, add) in plan {
        let existing = ferrum_kernel::load_calendar(&mut tx, ctx.tenant, &name).await?;
        let mut body = match existing {
            Some(cal) => json!(cal),
            None if name == CREATED => {
                json!({"zone": zone.as_deref().filter(|z| z.parse::<chrono_tz::Tz>().is_ok()).unwrap_or("UTC"), "weekends": [], "holidays": []})
            }
            // A calendar the old rows named that does not exist never opened or closed anything.
            None => continue,
        };
        if !body["exceptions"].is_object() {
            body["exceptions"] = json!({});
        }
        for (date, x) in add {
            if body["exceptions"].get(&date).is_none() {
                body["exceptions"][&date] = x;
                written += 1;
            }
        }
        puts.push((name, body));
    }
    tx.commit().await?;
    for (name, body) in &puts {
        k.put_logic_calendar(ctx, name, body).await?;
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    if let (true, Some((id, _))) = (created, &settings) {
        k.set_values_tx(ctx, &mut tx, "BookingSettings", *id, obj(json!({"calendar": CREATED}))).await?;
    }
    sqlx::query("insert into bookings_calendar_moved (tenant_id) values ($1) on conflict do nothing").bind(ctx.tenant).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(written)
}
