//! Bookings: appointment scheduling — services, staff and resources, availability, a public booking
//! page, reminders, reschedules and paid bookings.
//!
//! * `engine`: the pure availability engine (time zones, DST, buffers, seats, resources, fairness).
//! * `calendar`: its inputs from the database and the row locks that serialize bookings.
//! * `booking`: the Booking hook and actions (confirm, reschedule, cancel, complete, no-show,
//!   invoice, guest link) and the class waitlist.
//! * `setup`: hooks on services (`find_slots`), staff (calendar feed links), time off, settings.
//! * `site`: the public booking page endpoints; `notify`: notifications and reminders, each sent
//!   once, from maintenance; `reports`: volume, attendance and utilization.
//!
//! Hooks are registered per entity, so a tenant without the `bookings` package never runs them.

mod booking;
mod calendar;
pub mod engine;
mod ics;
mod notify;
mod reports;
mod settings;
mod setup;
mod site;
mod slots;
mod token;

use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Doc, Error, HookRegistry, Kernel, Result};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/packages.rs"));

/// An instant (stored as `timestamptz`).
pub type Instant = DateTime<Utc>;

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

/// Registers this app's document hooks and its public endpoints.
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("Booking", Arc::new(booking::BookingHook))
        .register("BookingService", Arc::new(setup::ServiceHook))
        .register("BookingStaff", Arc::new(setup::StaffHook))
        .register("StaffTimeOff", Arc::new(setup::TimeOffHook))
        .register("BookingSettings", Arc::new(setup::SettingsHook))
        .add_site("bookings", Arc::new(site::BookingSite));
}

/// This app's reports.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(reports::BookingReports)]
}

/// Periodic upkeep: every minute, on one replica at a time (advisory lock); must be idempotent and cheap.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    maintenance_at(k, Utc::now()).await
}

/// [`maintenance`] as of `now` (tests move the clock): per tenant with bookings, send change
/// notices and due reminders, close waitlists of started classes, fill freed class seats and
/// purge expired guest links. A failing tenant is logged and does not stop the others; the first
/// error is returned.
pub async fn maintenance_at(k: &Kernel, now: Instant) -> Result<()> {
    let mut tx = k.db.system_tx().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut first_error: Option<Error> = None;
    for tenant in tenants {
        let schema = match k.registry.schema(tenant).await {
            Ok(s) if s.packages.iter().any(|p| p == "bookings") => s,
            Ok(_) => continue,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };
        match notify::run(k, tenant, schema, now).await {
            Ok(round) if round != notify::Round::default() => tracing::info!(%tenant, ?round, "bookings maintenance"),
            Ok(_) => {}
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// RFC 3339 UTC with milliseconds, the kernel's canonical date-time form.
pub(crate) fn ts(t: Instant) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn time(doc: &Doc, k: &str) -> Option<Instant> {
    ferrum_domain::util::s(doc, k).and_then(|v| ferrum_kernel::value::parse_datetime(v).ok())
}

/// A whole-number field (missing or empty reads as 0).
pub(crate) fn int(doc: &Doc, k: &str) -> i64 {
    doc.get(k).and_then(Value::as_i64).unwrap_or(0)
}

pub(crate) fn obj(v: Value) -> Doc {
    match v {
        Value::Object(m) => m,
        _ => Doc::new(),
    }
}

pub(crate) fn doc_id(doc: &Doc) -> Result<Uuid> {
    ferrum_domain::util::id(doc, "id").ok_or_else(|| Error::internal("record without id"))
}

/// `Mon 2 Nov 2026, 10:00 (Asia/Kolkata)`.
pub(crate) fn when(t: Instant, tz: Tz) -> String {
    format!("{} ({})", t.with_timezone(&tz).format("%a %-d %b %Y, %H:%M"), tz.name())
}

#[cfg(test)]
mod tests {
    #[test]
    fn packages_are_valid() {
        let mut all = ferrum_domain::packages().expect("platform packages");
        all.extend(super::packages().expect("app packages"));
        let catalog = ferrum_meta::Catalog::from_packages(all).unwrap_or_else(|e| panic!("{e}"));
        let schema = ferrum_meta::effective::build(&catalog, &["bookings".to_string()], &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert!(schema.warnings.is_empty(), "{:?}", schema.warnings);
        assert!(schema.entity("BookingSettings").is_some_and(|e| e.secret_fields().count() == 1));
        let booking = schema.entity("Booking").expect("Booking");
        assert_eq!(booking.table, "e_booking");
        for action in ["confirm", "reschedule", "cancel_booking", "complete", "mark_no_show", "make_sales_invoice", "guest_link"] {
            assert!(booking.def.actions.iter().any(|a| a.name == action), "{action}");
        }
        assert!(schema.roles.iter().any(|r| r.name == "Bookings Manager") && schema.roles.iter().any(|r| r.name == "Bookings User"));
        assert!(super::MANIFEST.contains("packages: [bookings]"));
    }

    #[test]
    fn times_read_in_a_zone() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-11-02T04:30:00Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(super::when(t, "Asia/Kolkata".parse().unwrap()), "Mon 2 Nov 2026, 10:00 (Asia/Kolkata)");
        assert_eq!(super::ts(t), "2026-11-02T04:30:00.000Z");
    }
}
