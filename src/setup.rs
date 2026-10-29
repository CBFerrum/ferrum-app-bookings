//! Hooks on the setup records: services (validation, `find_slots`), staff (time zone and working
//! hours, calendar feed links), time off (who may enter it, bookings it collides with) and settings.

use crate::calendar;
use crate::engine::{self, Rules};
use crate::settings;
use crate::{doc_id, int, obj, slots, time, token, ts};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use chrono_tz::Tz;
use ferrum_domain::util::{id, rows, s, sys};
use ferrum_kernel::{Access, Action, Ctx, Doc, DocHook, Error, FieldError, HookCtx, Result};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

/// Rows of `table` whose `field` repeats an earlier row's value.
fn duplicates(doc: &Doc, table: &str, field: &str, message: &str) -> Vec<FieldError> {
    let mut seen = HashSet::new();
    rows(doc, table)
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let v = match r.get(field)? {
                Value::Null => return None,
                Value::String(v) => v.trim().to_lowercase(),
                other => other.to_string(),
            };
            (!seen.insert(v)).then(|| FieldError::new(field, message).in_row(table, i))
        })
        .collect()
}

fn param(params: &Value, key: &str) -> Option<String> {
    match params.get(key)? {
        Value::String(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub struct ServiceHook;

#[async_trait]
impl DocHook for ServiceHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = duplicates(doc, "staff", "staff", "is listed twice");
        errors.extend(duplicates(doc, "resources", "resource", "is listed twice"));
        errors.extend(duplicates(doc, "questions", "question", "is asked twice"));
        for (i, q) in rows(doc, "questions").iter().enumerate() {
            if s(q, "answer_type") == Some("Choice") && s(q, "options").is_none_or(|o| o.lines().all(|l| l.trim().is_empty())) {
                errors.push(FieldError::new("options", "list the choices, one per line").in_row("questions", i));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    /// `find_slots {from?, to?, time_zone?, staff?, seats?}`: bookable starts from now on (any
    /// distance ahead), as staff and agents may book them.
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "find_slots" {
            return Ok(None);
        }
        let tenant = hc.ctx.tenant;
        let svc = calendar::service(hc.tx, tenant, doc_id(doc)?).await?.ok_or_else(|| Error::NotFound("Booking Service".into()))?;
        if !svc.active {
            return Err(Error::invalid(format!("{} is not taking bookings", svc.name)));
        }
        let st = settings::load(hc.tx, tenant).await?;
        let default_tz = calendar::zone(&hc.ctx.schema.timezone).unwrap_or(Tz::UTC);
        let q = slots::parse(|k| param(params, k), default_tz)?;
        let rules = Rules { earliest: Some(Utc::now()), latest: None, on_grid: true };
        Ok(Some(slots::find(hc.tx, tenant, &svc, &st, &q, rules, None).await?))
    }
}

pub struct StaffHook;

/// Managers (every staff record writable) or the staff member themselves.
fn manager_or_self(ctx: &Ctx, staff: &Doc) -> Result<()> {
    let manager = ctx.is_system || ctx.perms.access("BookingStaff", Action::Write) == Access::All;
    let me = s(staff, "email").is_some_and(|e| e.eq_ignore_ascii_case(ctx.email.trim()));
    if manager || me { Ok(()) } else { Err(Error::Forbidden("only the staff member or a bookings manager can do this".into())) }
}

/// A signed calendar feed link for a staff member, valid for a year.
async fn feed_link(hc: &mut HookCtx<'_>, staff: Uuid, version: i64) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let key = settings::signing_key(hc.kernel, hc.ctx, hc.tx).await?;
    let st = settings::load(hc.tx, tenant).await?;
    let root = settings::site_root(hc.tx, tenant).await?;
    let expires = Utc::now() + Duration::days(365);
    let sig = token::feed_signature(&key, tenant, staff, version, expires.timestamp());
    let path = format!("{root}/feeds/{staff}.ics?v={version}&exp={}&sig={sig}", expires.timestamp());
    Ok(json!({
        "url": st.absolute(&path),
        "path": path,
        "expires_at": ts(expires),
        "note": "Subscribe to it in a calendar app. Anyone with the link sees these bookings: keep it private (Reset Calendar Feed Link retires it).",
    }))
}

#[async_trait]
impl DocHook for StaffHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        if s(doc, "time_zone").is_some_and(|z| calendar::zone(z).is_none()) {
            errors.push(FieldError::new("time_zone", "is not an IANA time zone (e.g. Europe/London)"));
        }
        let mut parsed: Vec<(usize, engine::Hours)> = Vec::new();
        for (i, r) in rows(doc, "working_hours").iter().enumerate() {
            let from = s(r, "start_time").and_then(engine::parse_hhmm);
            let to = s(r, "end_time").and_then(engine::parse_hhmm);
            match (s(r, "weekday").and_then(engine::weekday), from, to) {
                (_, None, _) => errors.push(FieldError::new("start_time", "use HH:MM (00:00 to 23:59)").in_row("working_hours", i)),
                (_, _, None) => errors.push(FieldError::new("end_time", "use HH:MM (00:00 to 24:00)").in_row("working_hours", i)),
                (Some(weekday), Some(from), Some(to)) if to > from => parsed.push((i, engine::Hours { weekday, from, to })),
                (Some(_), Some(_), Some(_)) => errors.push(FieldError::new("end_time", "must be after the start time").in_row("working_hours", i)),
                (None, ..) => {}
            }
        }
        for (n, (i, a)) in parsed.iter().enumerate() {
            if parsed[..n].iter().any(|(_, b)| a.weekday == b.weekday && a.from < b.to && b.from < a.to) {
                errors.push(FieldError::new("start_time", "overlaps another interval of the same day").in_row("working_hours", *i));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    /// Bookings and time off keep the staff member's email (it grants them access) and name:
    /// edits follow them there.
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let Some(old) = old else { return Ok(()) };
        if old.get("email") == doc.get("email") && old.get("display_name") == doc.get("display_name") {
            return Ok(());
        }
        for table in ["e_booking", "e_staff_time_off"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("update {table} set staff_email = $3, staff_name = $4 where tenant_id = $1 and staff = $2")))
                .bind(hc.ctx.tenant)
                .bind(doc_id(doc)?)
                .bind(s(doc, "email"))
                .bind(s(doc, "display_name"))
                .execute(&mut **hc.tx)
                .await?;
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        let staff = doc_id(doc)?;
        match name {
            "calendar_feed" => {
                manager_or_self(hc.ctx, doc)?;
                Ok(Some(feed_link(hc, staff, int(doc, "feed_version").max(1)).await?))
            }
            "rotate_calendar_feed" => {
                manager_or_self(hc.ctx, doc)?;
                let version = int(doc, "feed_version").max(1) + 1;
                hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "BookingStaff", staff, obj(json!({"feed_version": version}))).await?;
                Ok(Some(feed_link(hc, staff, version).await?))
            }
            _ => Ok(None),
        }
    }
}

pub struct TimeOffHook;

#[async_trait]
impl DocHook for TimeOffHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let Some(staff) = id(doc, "staff") else { return Ok(()) };
        let Ok(member) = hc.kernel.get_tx(&sys(hc.ctx), hc.tx, "BookingStaff", staff, false).await else { return Ok(()) };
        doc.insert("staff_name".into(), member.get("display_name").cloned().unwrap_or(Value::Null));
        doc.insert("staff_email".into(), member.get("email").cloned().unwrap_or(Value::Null));
        if let (Some(from), Some(to)) = (time(doc, "from_time"), time(doc, "to_time")) {
            let clashes: i64 = sqlx::query_scalar(
                "select count(*) from e_booking where tenant_id = $1 and staff = $2 and status in ('Pending', 'Confirmed') and start_time < $4 and end_time > $3",
            )
            .bind(hc.ctx.tenant)
            .bind(staff)
            .bind(from)
            .bind(to)
            .fetch_one(&mut **hc.tx)
            .await?;
            doc.insert("conflicting_bookings".into(), json!(clashes));
        }
        Ok(())
    }

    /// Staff without the manager's reach enter time off for themselves only.
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let ctx = hc.ctx;
        if ctx.is_system || ctx.perms.access("StaffTimeOff", Action::Write) == Access::All {
            return Ok(());
        }
        if s(doc, "staff_email").is_some_and(|e| e.eq_ignore_ascii_case(ctx.email.trim())) {
            Ok(())
        } else {
            Err(Error::Forbidden("you can enter time off for yourself only".into()))
        }
    }
}

pub struct SettingsHook;

#[async_trait]
impl DocHook for SettingsHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let errors = duplicates(doc, "reminders", "minutes_before", "is already a reminder");
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_rows_are_found() {
        let doc = obj(
            json!({"staff": [{"staff": "a"}, {"staff": "b"}, {"staff": "A"}], "reminders": [{"minutes_before": 60}, {"minutes_before": 1440}, {"minutes_before": 60}, {}]}),
        );
        let dups = duplicates(&doc, "staff", "staff", "twice");
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].row, Some(("staff".to_string(), 2)));
        let dups = duplicates(&doc, "reminders", "minutes_before", "again");
        assert_eq!(dups.iter().map(|e| e.row.clone()).collect::<Vec<_>>(), vec![Some(("reminders".to_string(), 2))], "numbers compare too; empty rows do not");
        assert_eq!(param(&json!({"seats": 2, "from": " 2026-11-02 ", "x": true, "e": ""}), "seats").as_deref(), Some("2"));
        assert_eq!(param(&json!({"from": " 2026-11-02 "}), "from").as_deref(), Some("2026-11-02"));
        assert_eq!(param(&json!({"e": ""}), "e"), None);
    }
}
