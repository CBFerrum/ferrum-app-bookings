//! Booking Settings (one per workspace, defaults when none was saved), the HMAC key in its secret
//! field, and the public links built from them.

use crate::Instant;
use chrono::Duration;
use ferrum_domain::util::sys;
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Settings {
    pub company: Option<Uuid>,
    pub public_page: bool,
    pub approval_required: bool,
    pub min_notice: Duration,
    pub max_advance: Duration,
    pub interval: Duration,
    pub cancel_window: Duration,
    pub reschedule_window: Duration,
    /// Reminder offsets in minutes, largest first, without duplicates.
    pub reminders: Vec<i64>,
    pub public_url: Option<String>,
    pub manage_page_url: Option<String>,
    /// The business's working calendar (Admin → Calendars): nobody is booked outside its open time.
    pub calendar: Option<String>,
}

type Row = (Uuid, Option<Uuid>, bool, bool, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<String>);

pub async fn load(tx: &mut Tx, tenant: Uuid) -> Result<Settings> {
    let row: Option<Row> = sqlx::query_as(
        "select id, company, public_page, approval_required, min_notice_minutes, max_advance_days, slot_interval_minutes, \
         cancellation_window_hours, reschedule_window_hours, public_url, manage_page_url from e_booking_settings where tenant_id = $1 limit 1",
    )
    .bind(tenant)
    .fetch_optional(&mut **tx)
    .await?;
    let (id, company, public_page, approval, notice, advance, interval, cancel, reschedule, public_url, manage) = match row {
        Some(r) => r,
        None => (Uuid::nil(), None, false, false, None, None, None, None, None, None, None),
    };
    let mut reminders: Vec<i64> = if id.is_nil() {
        vec![]
    } else {
        sqlx::query_scalar("select minutes_before from e_booking_settings__reminders where tenant_id = $1 and parent_id = $2 and minutes_before > 0")
            .bind(tenant)
            .bind(id)
            .fetch_all(&mut **tx)
            .await?
    };
    reminders.sort_unstable_by(|a, b| b.cmp(a));
    reminders.dedup();
    let calendar: Option<String> = if id.is_nil() {
        None
    } else {
        sqlx::query_scalar("select calendar from e_booking_settings where tenant_id = $1 and id = $2").bind(tenant).bind(id).fetch_one(&mut **tx).await?
    };
    let url = |v: Option<String>| v.map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty());
    Ok(Settings {
        company,
        public_page,
        approval_required: approval,
        min_notice: Duration::minutes(notice.unwrap_or(120).max(0)),
        max_advance: Duration::days(advance.unwrap_or(60).max(1)),
        interval: Duration::minutes(interval.unwrap_or(30).max(5)),
        cancel_window: Duration::hours(cancel.unwrap_or(24).max(0)),
        reschedule_window: Duration::hours(reschedule.unwrap_or(24).max(0)),
        reminders,
        public_url: url(public_url),
        manage_page_url: url(manage),
        calendar: calendar.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
    })
}

impl Settings {
    /// The rules that hold for `svc`: each of its own (notice, booking ahead, cancel and reschedule
    /// windows, approval, reminders), or this workspace's where it left one empty. Every caller
    /// deciding by these rules goes through here.
    pub fn for_service(&self, svc: &crate::calendar::ServiceInfo) -> Settings {
        let r = &svc.rules;
        let mut st = self.clone();
        if let Some(m) = r.min_notice_minutes {
            st.min_notice = Duration::minutes(m.max(0));
        }
        if let Some(d) = r.max_advance_days {
            st.max_advance = Duration::days(d.max(1));
        }
        if let Some(h) = r.cancellation_window_hours {
            st.cancel_window = Duration::hours(h.max(0));
        }
        if let Some(h) = r.reschedule_window_hours {
            st.reschedule_window = Duration::hours(h.max(0));
        }
        if let Some(a) = r.approval_required {
            st.approval_required = a;
        }
        if !r.reminders.is_empty() {
            st.reminders = r.reminders.clone();
            st.reminders.sort_unstable_by(|a, b| b.cmp(a));
            st.reminders.dedup();
        }
        st
    }

    /// Starts a guest may book from `now`.
    pub fn public_rules(&self, now: Instant) -> crate::engine::Rules {
        crate::engine::Rules { earliest: Some(now + self.min_notice), latest: Some(now + self.max_advance), on_grid: true }
    }

    /// The self-service link for a guest token: the business's own page when it set one, else the
    /// booking page the interface serves (`/bookings/book/{workspace}/manage/{token}`).
    pub fn manage_link(&self, root: &str, token: &str) -> String {
        match &self.manage_page_url {
            Some(page) => format!("{page}{}token={token}", if page.contains('?') { '&' } else { '?' }),
            None => self.absolute(&format!("{}/manage/{token}", page_root(root))),
        }
    }

    /// `path` on this server: absolute when the server address is set.
    pub fn absolute(&self, path: &str) -> String {
        match &self.public_url {
            Some(base) => format!("{base}{path}"),
            None => path.to_string(),
        }
    }
}

/// `/bookings/book/{workspace}`: where the booking page is, from the root of its endpoints
/// (`/api/site/{workspace}/bookings`).
fn page_root(api_root: &str) -> String {
    let workspace = api_root.strip_prefix("/api/site/").and_then(|r| r.split('/').next()).unwrap_or_default();
    format!("/bookings/book/{workspace}")
}

/// `/api/site/{tenant slug}/bookings`: the root of the public endpoints.
pub async fn site_root(tx: &mut Tx, tenant: Uuid) -> Result<String> {
    let slug: String = sqlx::query_scalar("select slug from tenant where id = $1").bind(tenant).fetch_one(&mut **tx).await?;
    Ok(format!("/api/site/{slug}/bookings"))
}

/// The HMAC key signing calendar feed links: the settings' secret field, created on first use
/// (inside the caller's transaction). Without a configured secret key the kernel refuses secrets.
pub async fn signing_key(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<Vec<u8>> {
    let tenant = ctx.tenant;
    let settings: Option<Uuid> =
        sqlx::query_scalar("select id from e_booking_settings where tenant_id = $1 limit 1 for update").bind(tenant).fetch_optional(&mut **tx).await?;
    if let Some(id) = settings
        && let Some(key) = k.secret(tx, tenant, "BookingSettings", id, "signing_secret").await?
    {
        return Ok(key.into_bytes());
    }
    let fresh = hex::encode(rand::random::<[u8; 32]>());
    let mut values = Doc::new();
    values.insert("signing_secret".into(), json!(fresh));
    let engine = sys(ctx);
    let written = match settings {
        Some(id) => k.update_tx(&engine, tx, "BookingSettings", id, values, None).await,
        None => k.insert_tx(&engine, tx, "BookingSettings", values).await,
    };
    match written {
        Ok(_) => Ok(fresh.into_bytes()),
        // Another request created the settings at the same moment: run again and read theirs.
        Err(Error::Conflict(_)) => Err(Error::Retryable("booking settings were created concurrently".into())),
        Err(e) => Err(e),
    }
}

/// The key, when one exists (public requests never create it).
pub async fn existing_key(k: &Kernel, tx: &mut Tx, tenant: Uuid) -> Result<Option<Vec<u8>>> {
    let settings: Option<Uuid> =
        sqlx::query_scalar("select id from e_booking_settings where tenant_id = $1 limit 1").bind(tenant).fetch_optional(&mut **tx).await?;
    match settings {
        Some(id) => Ok(k.secret(tx, tenant, "BookingSettings", id, "signing_secret").await?.map(String::into_bytes)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(page: Option<&str>, url: Option<&str>) -> Settings {
        Settings {
            company: None,
            public_page: true,
            approval_required: false,
            min_notice: Duration::minutes(120),
            max_advance: Duration::days(60),
            interval: Duration::minutes(30),
            cancel_window: Duration::hours(24),
            reschedule_window: Duration::hours(24),
            reminders: vec![],
            public_url: url.map(str::to_string),
            manage_page_url: page.map(str::to_string),
            calendar: None,
        }
    }

    #[test]
    fn a_service_rule_wins_and_an_empty_one_falls_back() {
        use crate::calendar::{ServiceInfo, ServiceRules};
        let ws = Settings { reminders: vec![1440, 60], ..settings(None, None) };
        let svc = |rules: ServiceRules| ServiceInfo {
            id: Uuid::nil(),
            name: "S".into(),
            active: true,
            description: None,
            duration: 60,
            before: 0,
            after: 0,
            interval: None,
            capacity: 1,
            price: Default::default(),
            item: None,
            location_type: "In Person".into(),
            location: None,
            staff: vec![],
            resources: vec![],
            questions: vec![],
            guest_repeats: 0,
            rules,
        };
        let same = ws.for_service(&svc(ServiceRules::default()));
        assert_eq!(
            (same.min_notice, same.max_advance, same.cancel_window, same.reschedule_window, same.approval_required, same.reminders.clone()),
            (ws.min_notice, ws.max_advance, ws.cancel_window, ws.reschedule_window, ws.approval_required, ws.reminders.clone()),
            "empty: every rule is the workspace's"
        );
        let own = ws.for_service(&svc(ServiceRules {
            min_notice_minutes: Some(0),
            max_advance_days: Some(365),
            cancellation_window_hours: Some(48),
            reschedule_window_hours: Some(2),
            approval_required: Some(true),
            reminders: vec![30, 2880, 30],
        }));
        assert_eq!(own.min_notice, Duration::zero(), "notice");
        assert_eq!(own.max_advance, Duration::days(365), "how far ahead");
        assert_eq!(own.cancel_window, Duration::hours(48), "cancellation window");
        assert_eq!(own.reschedule_window, Duration::hours(2), "reschedule window");
        assert!(own.approval_required, "approval");
        assert_eq!(own.reminders, [2880, 30], "reminders, largest first, once each");
        let now = chrono::Utc::now();
        assert_eq!(own.public_rules(now).earliest, Some(now), "the guest rules follow");
        // A service may also turn approval off where the workspace has it on.
        let strict = Settings { approval_required: true, ..ws.clone() };
        assert!(!strict.for_service(&svc(ServiceRules { approval_required: Some(false), ..Default::default() })).approval_required);
    }

    #[test]
    fn links_point_at_the_page_or_the_server() {
        let root = "/api/site/spa/bookings";
        assert_eq!(settings(None, None).manage_link(root, "tok"), "/bookings/book/spa/manage/tok");
        assert_eq!(settings(None, Some("https://erp.example.com")).manage_link(root, "tok"), "https://erp.example.com/bookings/book/spa/manage/tok");
        assert_eq!(settings(Some("https://spa.example.com/book"), None).manage_link(root, "tok"), "https://spa.example.com/book?token=tok");
        assert_eq!(settings(Some("https://spa.example.com/b?lang=en"), None).manage_link(root, "tok"), "https://spa.example.com/b?lang=en&token=tok");
        let now = chrono::Utc::now();
        let r = settings(None, None).public_rules(now);
        assert_eq!((r.earliest, r.latest, r.on_grid), (Some(now + Duration::minutes(120)), Some(now + Duration::days(60)), true));
    }
}
