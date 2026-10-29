//! Free slots of a service over a range of days in the asker's time zone: the shared read behind
//! the public `/slots` endpoints and the `find_slots` action (staff, AI agents).

use crate::calendar::{self, ServiceInfo};
use crate::engine::{self, Rules};
use crate::settings::Settings;
use crate::ts;
use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use ferrum_kernel::{Error, Result, Tx};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Longest range one request may cover.
pub const MAX_DAYS: i64 = 62;

#[derive(Debug, Clone)]
pub struct Query {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub tz: Tz,
    pub staff: Option<Uuid>,
    pub seats: i64,
}

/// Reads `from`, `to` (dates, default: the next 7 days from today in the zone), `time_zone`
/// (IANA, default `default_tz`), `staff` and `seats` through `get`.
pub fn parse(get: impl Fn(&str) -> Option<String>, default_tz: Tz) -> Result<Query> {
    let tz = match get("time_zone").or_else(|| get("tz")) {
        Some(z) => calendar::zone(&z).ok_or_else(|| Error::field("time_zone", "is not an IANA time zone (e.g. Europe/Paris)"))?,
        None => default_tz,
    };
    let date = |k: &str| -> Result<Option<NaiveDate>> { get(k).map(|v| ferrum_kernel::value::parse_date(&v).map_err(|m| Error::field(k, m))).transpose() };
    let today = Utc::now().with_timezone(&tz).date_naive();
    let from = date("from")?.unwrap_or(today);
    let to = date("to")?.unwrap_or(from + chrono::Duration::days(6));
    if to < from {
        return Err(Error::field("to", "is before from"));
    }
    if (to - from).num_days() >= MAX_DAYS {
        return Err(Error::field("to", format!("ask for at most {MAX_DAYS} days at a time")));
    }
    let staff = get("staff").map(|v| v.parse::<Uuid>().map_err(|_| Error::field("staff", "is not a staff id"))).transpose()?;
    let seats = match get("seats") {
        None => 1,
        Some(v) => v.trim().parse::<i64>().ok().filter(|n| (1..=1000).contains(n)).ok_or_else(|| Error::field("seats", "must be a whole number from 1"))?,
    };
    Ok(Query { from, to, tz, staff, seats })
}

/// The bookable starts, grouped by local date in `q.tz`; `exclude` leaves out a booking being moved.
pub async fn find(tx: &mut Tx, tenant: Uuid, svc: &ServiceInfo, st: &Settings, q: &Query, rules: Rules, exclude: Option<Uuid>) -> Result<Value> {
    if q.seats > svc.capacity {
        return Err(Error::field("seats", format!("{} takes at most {} seat(s) per booking", svc.name, svc.capacity)));
    }
    let candidates = match q.staff {
        Some(s) if svc.staff.contains(&s) => vec![s],
        Some(_) => return Err(Error::field("staff", format!("does not provide {}", svc.name))),
        None => svc.staff.clone(),
    };
    let staff = calendar::staff(tx, tenant, &candidates).await?;
    let window = engine::local_days(q.tz, q.from, q.to);
    let mut world = calendar::world(tx, tenant, svc, &staff, st, window, exclude).await?;
    world.rules = rules;
    let name = |id: Uuid| staff.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
    let mut days: BTreeMap<NaiveDate, Vec<Value>> = BTreeMap::new();
    for slot in world.slots(window, q.seats) {
        let local = slot.start.with_timezone(&q.tz);
        let seats_left = slot.options.iter().map(|o| o.seats_left).max().unwrap_or(0);
        days.entry(local.date_naive()).or_default().push(json!({
            "start": ts(slot.start),
            "end": ts(slot.end),
            "time": local.format("%H:%M").to_string(),
            "seats_left": seats_left,
            "staff": slot.options.iter().map(|o| json!({"id": o.staff, "name": name(o.staff)})).collect::<Vec<_>>(),
        }));
    }
    Ok(json!({
        "service": {"id": svc.id, "name": svc.name, "duration_minutes": svc.duration, "seats": svc.capacity},
        "time_zone": q.tz.name(),
        "from": q.from,
        "to": q.to,
        "days": days.into_iter().map(|(date, slots)| json!({"date": date, "slots": slots})).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> Result<Query> {
        let map: std::collections::HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        parse(|k| map.get(k).cloned(), Tz::UTC)
    }

    #[test]
    fn queries_are_validated() {
        let ok = q(&[("from", "2026-11-02"), ("to", "2026-11-05"), ("time_zone", "Asia/Kolkata"), ("seats", "2")]).unwrap();
        assert_eq!((ok.from.to_string(), ok.to.to_string(), ok.tz.name(), ok.seats), ("2026-11-02".into(), "2026-11-05".into(), "Asia/Kolkata", 2));
        let default = q(&[("from", "2026-11-02")]).unwrap();
        assert_eq!((default.to.to_string(), default.tz.name(), default.seats, default.staff), ("2026-11-08".into(), "UTC", 1, None));
        assert!(q(&[("from", "2026-11-05"), ("to", "2026-11-02")]).is_err());
        assert!(q(&[("from", "2026-01-01"), ("to", "2026-03-04")]).is_err(), "63 days");
        assert!(q(&[("from", "2026-01-01"), ("to", "2026-03-03")]).is_ok(), "62 days");
        assert!(q(&[("time_zone", "Mars/Olympus")]).is_err());
        assert!(q(&[("seats", "0")]).is_err());
        assert!(q(&[("staff", "nobody")]).is_err());
        assert!(q(&[("from", "02/11/2026")]).is_err());
    }
}
