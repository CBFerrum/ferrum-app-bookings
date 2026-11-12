//! Bookings end to end against a real PostgreSQL (`FERRUM_BOOKINGS_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_bookings_test`). The database is reset first.
//!
//! A spa in Pune (workspace zone Asia/Kolkata) sets up its calendar: Asha works in Pune with a
//! lunch break, Ben in London; massages need one of two rooms and a clean-up buffer; yoga is a
//! class in the studio. Guests book through the public endpoints exactly as a booking page would
//! (any staff, intake answers, honeypot, refusals), manage their bookings by token (reschedule,
//! cancel, windows), race each other for the same slot and the last class seats, join and leave
//! waitlists. Maintenance sends every notice and reminder exactly once — in-app, then by email once
//! the Mail app is installed — staff see their own bookings only, invoices bill the booked price
//! exactly once, calendar feeds are signed, and the reports add up.

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, Site, SiteRequest, SiteResponse};
use ferrum_meta::Catalog;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use uuid::Uuid;

const WEEK: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(doc: &Doc) -> Uuid {
    doc["id"].as_str().unwrap().parse().unwrap()
}

fn sid(doc: &Doc) -> String {
    doc["id"].as_str().unwrap().to_string()
}

fn rfc(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A wall-clock time on a local day in `tz`.
fn local(tz: Tz, day: NaiveDate, hhmm: &str) -> DateTime<Utc> {
    let (h, m) = hhmm.split_once(':').unwrap();
    tz.from_local_datetime(&day.and_hms_opt(h.parse().unwrap(), m.parse().unwrap(), 0).unwrap()).earliest().unwrap().with_timezone(&Utc)
}

/// The Mail app's `Communication` as far as Bookings uses it (the real app has no test seam in
/// this repository; its contract is its README and mail.yaml).
fn mail_stand_in() -> ferrum_meta::Package {
    let mut files = BTreeMap::new();
    files.insert("package.yaml".to_string(), "package: comms\nversion: 0.0.0-test\nlabel: Mail stand-in\ndepends: [core]\n".to_string());
    files.insert(
        "entities/mail.yaml".to_string(),
        r#"- name: Communication
  label: Communication
  kind: master
  title_field: subject
  fields:
    - {name: subject, type: data, max_length: 998}
    - {name: channel, type: select, options: [Email, SMS, Chat, Phone, Note, Meeting], default: Email, required: true}
    - {name: direction, type: select, options: [Sent, Received], default: Sent, required: true}
    - {name: recipients, type: text}
    - {name: content, type: text}
    - {name: reference_entity, type: data}
    - {name: reference_id, type: data}
    - {name: ics, type: text, read_only: true, hidden: true}
"#
        .to_string(),
    );
    ferrum_meta::catalog::load_package_files(&files).expect("mail stand-in")
}

async fn site_call(k: &Kernel, site: &Arc<dyn Site>, tenant: Uuid, method: &str, path: &str, body: Option<Value>, ip: &str) -> Result<SiteResponse, Error> {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let query: Map<String, Value> =
        query.split('&').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect();
    let req = SiteRequest {
        tenant,
        user: None,
        method: method.into(),
        path: path.into(),
        query,
        headers: Map::new(),
        body: body.map(|b| b.to_string().into_bytes()).unwrap_or_default(),
        client_ip: ip.into(),
    };
    site.handle(k, req).await
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    site: Arc<dyn Site>,
    calls: AtomicU32,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Doc {
        self.k.update(ctx, entity, id, obj(v), None).await.unwrap_or_else(|e| panic!("update {entity}: {e:?}"))
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn action(&self, ctx: &Ctx, entity: &str, id: Uuid, name: &str, params: Value) -> Result<Value, Error> {
        self.k.run_action(ctx, entity, id, name, params).await
    }
    async fn act(&self, ctx: &Ctx, entity: &str, id: Uuid, name: &str, params: Value) -> Value {
        self.action(ctx, entity, id, name, params).await.unwrap_or_else(|e| panic!("{entity}.{name}: {e:?}"))
    }
    async fn list(&self, ctx: &Ctx, entity: &str, filters: Value) -> Result<Vec<Doc>, Error> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        Ok(self.k.list(ctx, entity, &q).await?.rows)
    }
    async fn find_one(&self, entity: &str, filters: Value) -> Doc {
        let rows = self.list(&self.admin, entity, filters.clone()).await.unwrap();
        assert_eq!(rows.len(), 1, "{entity} {filters}");
        rows[0].clone()
    }
    /// A number from SQL with the tenant bound as $1 (and `id` as $2).
    async fn scalar(&self, sql: &str, id: Option<Uuid>) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).fetch_one(&mut *tx).await.unwrap()
    }
    async fn texts(&self, sql: &str, id: Option<Uuid>) -> Vec<String> {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).fetch_all(&mut *tx).await.unwrap()
    }
    /// A public request from its own address (the write throttle is per address).
    async fn site(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), Error> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        let r = site_call(&self.k, &self.site, self.tenant, method, path, body, &format!("10.9.{}.{}", n / 200, n % 200 + 1)).await?;
        assert_eq!(r.content_type, "application/json");
        assert!(r.headers.iter().any(|(k, v)| k == "cache-control" && v == "no-store"));
        Ok((r.status, serde_json::from_slice(&r.body).expect("json body")))
    }
    async fn get_pub(&self, path: &str) -> Value {
        self.site("GET", path, None).await.unwrap_or_else(|e| panic!("GET {path}: {e:?}")).1
    }
    async fn post_pub(&self, path: &str, body: Value) -> Result<Value, Error> {
        self.site("POST", path, Some(body)).await.map(|r| r.1)
    }
    async fn book(&self, body: Value) -> Result<Value, Error> {
        let (status, v) = self.site("POST", "/book", Some(body)).await?;
        assert_eq!(status, 201);
        Ok(v)
    }
    async fn raw(&self, path: &str) -> Result<SiteResponse, Error> {
        site_call(&self.k, &self.site, self.tenant, "GET", path, None, "10.8.0.1").await
    }
    async fn report(&self, ctx: &Ctx, name: &str, params: &[(&str, String)]) -> Value {
        let p: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        ferrum_domain::reports::run(&self.k, ctx, name, &p).await.unwrap_or_else(|e| panic!("report {name}: {e:?}"))
    }
    async fn user(&self, email: &str, roles: &[&str]) -> Ctx {
        let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
        self.k.load_ctx(self.tenant, id, None, "it".into()).await.expect("ctx")
    }
    async fn reload(&self, ctx: &Ctx) -> Ctx {
        self.k.load_ctx(self.tenant, ctx.user, None, "it".into()).await.expect("ctx")
    }
    async fn maintain(&self, now: DateTime<Utc>) {
        ferrum_app_bookings::maintenance_at(&self.k, now).await.expect("maintenance");
    }
    async fn notes(&self, user: &Ctx) -> Vec<String> {
        self.texts("select title from notification where tenant_id = $1 and user_id = $2 order by created_at, id", Some(user.user)).await
    }
    async fn mails_to(&self, email: &str) -> Vec<Doc> {
        // A fresh context: the Mail app may have been installed after `admin` was loaded.
        let admin = self.reload(&self.admin).await;
        let mut rows = self.list(&admin, "Communication", json!({"recipients": email})).await.unwrap();
        rows.sort_by(|a, b| a["created_at"].as_str().cmp(&b["created_at"].as_str()));
        rows
    }
}

/// Start times (`HH:MM` in the query's zone) a staff member is offered.
fn times_for(slots: &Value, staff: &str) -> Vec<String> {
    slots["days"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|d| d["slots"].as_array().unwrap().iter())
        .filter(|s| s["staff"].as_array().unwrap().iter().any(|x| x["id"] == json!(staff)))
        .map(|s| s["time"].as_str().unwrap().to_string())
        .collect()
}

fn all_starts(slots: &Value) -> Vec<String> {
    slots["days"].as_array().unwrap().iter().flat_map(|d| d["slots"].as_array().unwrap().iter()).map(|s| s["start"].as_str().unwrap().to_string()).collect()
}

/// A stand-in for an outside calendar's address, on this machine: answers each request with the
/// status and calendar it holds at the time, and counts the requests.
async fn calendar_stand_in(calendar: Arc<std::sync::Mutex<(u16, String)>>) -> (String, Arc<AtomicU32>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/calendar.ics", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicU32::new(0));
    let counted = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let _ = sock.read(&mut buf).await;
            counted.fetch_add(1, Ordering::SeqCst);
            let (status, body) = calendar.lock().unwrap().clone();
            let answer = format!("HTTP/1.1 {status} X\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = sock.write_all(answer.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (url, hits)
}

fn messages(e: &Error) -> String {
    match e {
        Error::Validation(list) => list.iter().map(|f| format!("{}: {}", f.field, f.message)).collect::<Vec<_>>().join(" | "),
        other => other.to_string(),
    }
}

fn token_in(content: &str) -> String {
    let at = content.find("token=").expect("a self-service link") + 6;
    content[at..at + 43].to_string()
}

/// The tests share one database that each of them resets: one at a time.
static DB: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn bookings_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_BOOKINGS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_bookings_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 24, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    // Bookings without its Commerce connector, which deposits_end_to_end tests.
    packages.extend(ferrum_app_bookings::packages().expect("app packages").into_iter().filter(|p| p.name() != "bookings_payments"));
    packages.push(mail_stand_in());
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    for source in ferrum_app_erp::reports() {
        ferrum_domain::reports::add_source(source);
    }
    ferrum_app_bookings::register(&mut hooks);
    for source in ferrum_app_bookings::reports() {
        ferrum_domain::reports::add_source(source);
    }
    // Private addresses allowed: the outside calendar's stand-in is on this machine.
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), webhooks_allow_private: true, ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    ferrum_app_erp::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "spa".into(),
            name: "Serenity Spa".into(),
            admin_email: "admin@spa.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["bookings".into()],
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Asia/Kolkata"})),
        })
        .await
        .expect("tenant");
    let session = k.login("spa", "admin@spa.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("bookings").cloned().expect("bookings site");
    let t = T { k: k.clone(), tenant, admin, site, calls: AtomicU32::new(0) };
    let a = t.admin.clone();
    let ist: Tz = "Asia/Kolkata".parse().unwrap();
    let london: Tz = "Europe/London".parse().unwrap();
    let today = Utc::now().with_timezone(&ist).date_naive();
    let (d, d1, d2) = (today + Duration::days(3), today + Duration::days(4), today + Duration::days(5));
    let at = |day: NaiveDate, hhmm: &str| local(ist, day, hhmm);

    // ───────────── books, items, people ─────────────
    let inr = t.find_one("Currency", json!({"code": "INR"})).await;
    let company = t.insert(&a, "Company", json!({"company_name": "Serenity Spa", "abbr": "SS", "default_currency": inr["id"]})).await;
    let cid = uid(&company);
    let output_tax = uid(&t.find_one("Account", json!({"account_name": "Output Tax", "company": cid.to_string()})).await);
    t.insert(
        &a,
        "TaxTemplate",
        json!({"title": "GST 18%", "company": cid.to_string(), "kind": "Sales", "is_default": true,
               "rows": [{"charge_type": "On Net Total", "account": output_tax.to_string(), "description": "GST 18%", "rate": "18"}]}),
    )
    .await;
    let services_group = t.find_one("ItemGroup", json!({"group_name": "Services"})).await;
    let nos = t.find_one("Uom", json!({"uom_name": "Nos"})).await;
    let item = |code: &str| json!({"item_code": code, "item_name": code, "item_group": services_group["id"], "stock_uom": nos["id"], "is_stock_item": false});
    let massage_item = uid(&t.insert(&a, "Item", item("MASSAGE-60")).await);
    let yoga_item = uid(&t.insert(&a, "Item", item("YOGA-DROP-IN")).await);
    let manager = t.user("manager@spa.test", &["Bookings Manager"]).await;
    let asha_user = t.user("asha@spa.test", &["Bookings User"]).await;
    let ben_user = t.user("ben@spa.test", &["Bookings User"]).await;
    let clerk = t.user("clerk@spa.test", &["Sales User"]).await;

    // ───────────── setup: settings, staff, resources, services ─────────────
    assert!(matches!(t.site("GET", "/services", None).await, Err(Error::NotFound(_))), "no settings: the page is closed");
    let settings = t
        .insert(
            &manager,
            "BookingSettings",
            json!({"company": cid.to_string(), "public_page": true, "min_notice_minutes": 120, "max_advance_days": 60, "slot_interval_minutes": 30,
                   "cancellation_window_hours": 24, "reschedule_window_hours": 12, "public_url": "https://erp.spa.test/",
                   "manage_page_url": "https://spa.test/manage", "reminders": [{"minutes_before": 1440}, {"minutes_before": 60}]}),
        )
        .await;
    let settings_id = uid(&settings);
    assert_eq!(settings["signing_secret"], Value::Null, "the feed key is made on first use");
    let dup = t.k.update(&manager, "BookingSettings", settings_id, obj(json!({"reminders": [{"minutes_before": 60}, {"minutes_before": 60}]})), None).await;
    assert!(matches!(&dup, Err(Error::Validation(e)) if e[0].message.contains("already a reminder")), "{dup:?}");

    let every = |from: &str, to: &str| WEEK.iter().map(|d| json!({"weekday": d, "start_time": from, "end_time": to})).collect::<Vec<_>>();
    let mut asha_hours = every("09:00", "13:00");
    asha_hours.extend(every("14:00", "18:00"));
    let asha = t
        .insert(
            &manager,
            "BookingStaff",
            json!({"display_name": "Asha Rao", "email": "Asha@Spa.test", "time_zone": "Asia/Kolkata", "working_hours": asha_hours}),
        )
        .await;
    let ben = t
        .insert(
            &manager,
            "BookingStaff",
            json!({"display_name": "Ben Carter", "email": "ben@spa.test", "time_zone": "Europe/London", "working_hours": every("08:00", "16:00")}),
        )
        .await;
    let (asha_id, ben_id) = (uid(&asha), uid(&ben));
    assert_eq!(asha["email"], json!("asha@spa.test"));
    for (hours, tz, why) in [
        (json!([{"weekday": "Monday", "start_time": "9:00", "end_time": "17:00"}]), "Asia/Kolkata", "HH:MM"),
        (json!([{"weekday": "Monday", "start_time": "17:00", "end_time": "09:00"}]), "Asia/Kolkata", "after the start"),
        (
            json!([{"weekday": "Monday", "start_time": "09:00", "end_time": "13:00"}, {"weekday": "Monday", "start_time": "12:00", "end_time": "15:00"}]),
            "Asia/Kolkata",
            "overlaps",
        ),
        (json!([]), "Mars/Base", "IANA"),
    ] {
        let e =
            t.k.insert(&manager, "BookingStaff", obj(json!({"display_name": "X", "email": "x@spa.test", "time_zone": tz, "working_hours": hours})))
                .await
                .unwrap_err();
        assert!(messages(&e).contains(why), "{why}: {}", messages(&e));
    }
    let temp = t.insert(&manager, "BookingStaff", json!({"display_name": "Temp", "email": "temp@spa.test"})).await;
    assert_eq!(temp["time_zone"], json!("Asia/Kolkata"), "staff default to the workspace's zone");
    t.k.delete(&manager, "BookingStaff", uid(&temp), None).await.unwrap();
    assert!(matches!(t.k.insert(&asha_user, "BookingStaff", obj(json!({"display_name": "Me", "email": "me@spa.test"}))).await, Err(Error::Forbidden(_))));

    let room_a = uid(&t.insert(&manager, "BookingResource", json!({"resource_name": "Room A", "capacity": 1})).await);
    let room_b = uid(&t.insert(&manager, "BookingResource", json!({"resource_name": "Room B", "capacity": 1})).await);
    let studio = uid(&t.insert(&manager, "BookingResource", json!({"resource_name": "Studio", "capacity": 1})).await);

    let massage = t
        .insert(
            &manager,
            "BookingService",
            json!({"service_name": "Deep Tissue Massage", "duration_minutes": 60, "buffer_after_minutes": 15, "price": "1200",
                   "item": massage_item.to_string(), "location_type": "In Person", "location_details": "12 Lake Road, Pune",
                   "staff": [{"staff": sid(&asha)}, {"staff": sid(&ben)}], "resources": [{"resource": room_a.to_string()}, {"resource": room_b.to_string()}],
                   "questions": [{"question": "Any allergies?", "answer_type": "Yes/No", "required": true},
                                 {"question": "Pressure", "answer_type": "Choice", "options": "Soft\nMedium\nFirm"}]}),
        )
        .await;
    let yoga = t
        .insert(
            &manager,
            "BookingService",
            json!({"service_name": "Morning Yoga", "duration_minutes": 60, "capacity": 3, "price": "500", "item": yoga_item.to_string(),
                   "location_type": "Video", "location_details": "https://meet.spa.test/yoga", "staff": [{"staff": sid(&asha)}],
                   "resources": [{"resource": studio.to_string()}]}),
        )
        .await;
    let (massage_id, yoga_id) = (uid(&massage), uid(&yoga));
    for (body, why) in [
        (json!({"service_name": "Paid", "price": "10", "staff": [{"staff": sid(&asha)}]}), "Item is required"),
        (json!({"service_name": "Twice", "staff": [{"staff": sid(&asha)}, {"staff": sid(&asha)}]}), "listed twice"),
        (json!({"service_name": "Nobody"}), "add at least one row"),
        (json!({"service_name": "Choosy", "staff": [{"staff": sid(&asha)}], "questions": [{"question": "Q", "answer_type": "Choice"}]}), "Options is required"),
    ] {
        let e = t.k.insert(&manager, "BookingService", obj(body)).await.unwrap_err();
        assert!(messages(&e).contains(why), "{why}: {}", messages(&e));
    }
    assert!(matches!(t.k.insert(&asha_user, "BookingService", obj(json!({"service_name": "Mine"}))).await, Err(Error::Forbidden(_))));

    // ───────────── the public page: services and free slots ─────────────
    let listing = t.get_pub("/services").await;
    assert_eq!((listing["time_zone"].clone(), listing["currency"].clone()), (json!("Asia/Kolkata"), json!("INR")));
    let offered = listing["services"].as_array().unwrap();
    assert_eq!(offered.iter().map(|s| s["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Deep Tissue Massage", "Morning Yoga"]);
    assert_eq!(offered[0]["price"], json!("1200"));
    assert_eq!(offered[0]["location"], json!("12 Lake Road, Pune"));
    assert_eq!(offered[0]["staff"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Asha Rao", "Ben Carter"]);
    assert_eq!(offered[0]["questions"][0], json!({"question": "Any allergies?", "type": "Yes/No", "options": [], "required": true}));
    assert_eq!((offered[1]["seats"].clone(), offered[1]["location"].clone()), (json!(3), Value::Null), "video links go to booked guests only");

    let slots_path = |svc: Uuid, from: NaiveDate, to: NaiveDate, tz: &str| format!("/slots?service={svc}&from={from}&to={to}&time_zone={tz}");
    let day_slots = t.get_pub(&slots_path(massage_id, d, d, "Asia/Kolkata")).await;
    assert_eq!(day_slots["days"][0]["date"], json!(d.to_string()));
    let asha_day = ["09:00", "09:30", "10:00", "10:30", "11:00", "11:30", "12:00", "14:00", "14:30", "15:00", "15:30", "16:00", "16:30", "17:00"];
    assert_eq!(times_for(&day_slots, &sid(&asha)), asha_day, "the lunch break and closing time leave no room");
    let ben_opens = local(london, d, "08:00").with_timezone(&ist).format("%H:%M").to_string();
    assert_eq!(times_for(&day_slots, &sid(&ben)).first(), Some(&ben_opens), "Ben's 08:00 in London, shown in Pune time");
    // A New York guest sees the same instants in New York time.
    let ny_slots = t.get_pub(&slots_path(massage_id, d - Duration::days(1), d, "America/New_York")).await;
    let ny: Tz = "America/New_York".parse().unwrap();
    let first = at(d, "09:00");
    let seen =
        ny_slots["days"].as_array().unwrap().iter().flat_map(|x| x["slots"].as_array().unwrap().iter()).find(|s| s["start"] == json!(rfc(first))).unwrap();
    assert_eq!(seen["time"], json!(first.with_timezone(&ny).format("%H:%M").to_string()));
    // Minimum notice: nothing within two hours from now.
    let soonest = Utc::now() + Duration::minutes(120);
    for s in all_starts(&t.get_pub(&slots_path(massage_id, today, today + Duration::days(1), "Asia/Kolkata")).await) {
        assert!(DateTime::parse_from_rfc3339(&s).unwrap() >= soonest, "{s} is inside the notice period");
    }
    for bad in [slots_path(massage_id, d, d + Duration::days(62), "Asia/Kolkata"), slots_path(massage_id, d, d, "Mars/Base")] {
        assert!(matches!(t.site("GET", &bad, None).await, Err(Error::Validation(_))), "{bad}");
    }
    assert!(matches!(t.site("GET", &slots_path(Uuid::now_v7(), d, d, "UTC"), None).await, Err(Error::NotFound(_))));

    // ───────────── outside calendars (FERRUM-BOOKINGS-004) ─────────────
    // Asha's own calendar elsewhere, read by its iCalendar address: the dentist from 10:00 to 11:00
    // on day d, a reminder marked free at noon, and a conference all of d1.
    let ics_at = |t: DateTime<Utc>| t.format("%Y%m%dT%H%M%SZ").to_string();
    let event = |uid: &str, from: DateTime<Utc>, to: DateTime<Utc>, more: &str| {
        format!("BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Private\r\n{more}END:VEVENT\r\n", ics_at(from), ics_at(to))
    };
    let feed = |events: String| format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//elsewhere//EN\r\n{events}END:VCALENDAR\r\n");
    let conference =
        format!("BEGIN:VEVENT\r\nUID:conf\r\nDTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\nEND:VEVENT\r\n", d1.format("%Y%m%d"), d2.format("%Y%m%d"));
    let dentist = event("dentist", at(d, "10:00"), at(d, "11:00"), "");
    let reminder = event("reminder", at(d, "12:00"), at(d, "12:30"), "TRANSP:TRANSPARENT\r\n");
    let calendar = Arc::new(std::sync::Mutex::new((200u16, feed(format!("{dentist}{reminder}{conference}")))));
    let (url, hits) = calendar_stand_in(calendar.clone()).await;
    t.update(&manager, "BookingStaff", asha_id, json!({"outside_calendar": url})).await;
    assert_eq!(t.get("BookingStaff", asha_id).await["outside_calendar"], json!(true), "the address is a secret: it opens her calendar");
    let now = Utc::now();
    t.maintain(now).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let read = t.get("BookingStaff", asha_id).await;
    assert!(read["outside_calendar_read"].is_string() && read["outside_calendar_error"].is_null(), "{read:?}");
    // The massage's 15 minutes after it count too, so 09:00 goes with the dentist's hour.
    let without_dentist: Vec<&str> = asha_day.iter().copied().filter(|h| !["09:00", "09:30", "10:00", "10:30"].contains(h)).collect();
    let asha_sid = sid(&asha);
    let asha_on = |from: NaiveDate, to: NaiveDate| {
        let (t, who, path) = (&t, &asha_sid, slots_path(massage_id, from, to, "Asia/Kolkata"));
        async move { times_for(&t.get_pub(&path).await, who) }
    };
    assert_eq!(asha_on(d, d1).await, without_dentist, "nothing on the conference day; the free reminder is free");
    assert_eq!(t.scalar("select count(*) from bookings_outside_busy where tenant_id = $1 and staff = $2", Some(asha_id)).await, 2, "times only");
    t.maintain(now + Duration::minutes(5)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1, "read every quarter of an hour");
    // Unreadable, it says why, and what was read before still counts.
    calendar.lock().unwrap().0 = 500;
    t.maintain(now + Duration::minutes(16)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(t.get("BookingStaff", asha_id).await["outside_calendar_error"], json!("it answered 500 Internal Server Error"));
    assert_eq!(asha_on(d, d).await, without_dentist);
    // The dentist cancelled: read again, the hour is offered once more.
    *calendar.lock().unwrap() = (200, feed(conference.clone()));
    t.maintain(now + Duration::minutes(32)).await;
    assert_eq!(asha_on(d, d).await, asha_day);
    assert!(t.get("BookingStaff", asha_id).await["outside_calendar_error"].is_null());
    // The address taken off her record, the conference no longer counts.
    t.update(&manager, "BookingStaff", asha_id, json!({"outside_calendar": null})).await;
    t.maintain(now + Duration::minutes(33)).await;
    assert_eq!(asha_on(d1, d1).await, asha_day);
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    // ───────────── internal booking, buffers, fair choice of staff ─────────────
    let meera = t.insert(&manager, "Customer", json!({"customer_name": "Meera Iyer", "customer_type": "Individual", "email": "meera@example.test"})).await;
    let b1 = t
        .insert(
            &manager,
            "Booking",
            json!({"service": massage_id.to_string(), "staff": sid(&asha), "start_time": rfc(at(d, "10:00")), "customer": sid(&meera)}),
        )
        .await;
    assert_eq!(
        (b1["status"].clone(), b1["source"].clone(), b1["staff_name"].clone(), b1["resource"].clone(), b1["guest_name"].clone(), b1["guest_email"].clone()),
        (json!("Confirmed"), json!("Internal"), json!("Asha Rao"), json!(room_a.to_string()), json!("Meera Iyer"), json!("meera@example.test"))
    );
    assert_eq!((b1["end_time"].clone(), b1["block_end"].clone()), (json!(rfc(at(d, "11:00"))), json!(rfc(at(d, "11:15")))));
    assert_eq!((b1["rate"].clone(), b1["amount"].clone(), b1["company"].clone()), (json!("1200"), json!("1200"), json!(cid.to_string())));
    assert_eq!(b1["session_key"], b1["id"], "a one-to-one booking is its own session");
    assert!(b1["number"].as_str().unwrap().starts_with("BKG-"));
    let morning: Vec<String> =
        times_for(&t.get_pub(&slots_path(massage_id, d, d, "Asia/Kolkata")).await, &sid(&asha)).into_iter().filter(|x| x.as_str() < "13:00").collect();
    assert_eq!(morning, ["11:30", "12:00"], "10:00 blocks until 11:15 with its clean-up buffer");

    let massage_body = |name: &str, start: DateTime<Utc>, staff: Option<&str>| {
        json!({"service": massage_id.to_string(), "start_time": rfc(start), "staff": staff, "name": name,
               "email": format!("{}@Example.test", name.split(' ').next().unwrap()), "time_zone": "America/New_York",
               "answers": {"Any allergies?": "no", "Pressure": "firm"}})
    };
    // Asha already has a booking that day, Ben none: "any staff" goes to Ben, in the first room.
    let jane = t.book(massage_body("Jane Doe", at(d, "15:00"), None)).await.unwrap();
    let jane_token = jane["token"].as_str().unwrap().to_string();
    assert_eq!(jane_token.len(), 43);
    assert_eq!((jane["booking"]["staff"]["name"].clone(), jane["booking"]["status"].clone()), (json!("Ben Carter"), json!("Confirmed")));
    assert!(jane["booking"]["when"].as_str().unwrap().ends_with("(America/New_York)"));
    assert_eq!(jane["booking"]["links"]["manage"], json!(format!("https://spa.test/manage?token={jane_token}")));
    assert_eq!(jane["booking"]["links"]["ics"], json!(format!("https://erp.spa.test/api/site/spa/bookings/manage/{jane_token}/booking.ics")));
    let jane_id = uid(&t.find_one("Booking", json!({"guest_email": "jane@example.test"})).await);
    let jane_doc = t.get("Booking", jane_id).await;
    assert_eq!((jane_doc["source"].clone(), jane_doc["resource"].clone()), (json!("Public"), json!(room_a.to_string())));
    let answers: Vec<(String, String)> =
        jane_doc["answers"].as_array().unwrap().iter().map(|r| (r["question"].as_str().unwrap().into(), r["answer"].as_str().unwrap().into())).collect();
    assert_eq!(answers, [("Any allergies?".to_string(), "No".to_string()), ("Pressure".into(), "Firm".into())]);
    let kim = t.book(massage_body("Kim Lee", at(d, "15:00"), None)).await.unwrap();
    let kim_token = kim["token"].as_str().unwrap().to_string();
    assert_eq!(kim["booking"]["staff"]["name"], json!("Asha Rao"));
    let kim_id = uid(&t.find_one("Booking", json!({"guest_email": "kim@example.test"})).await);
    assert_eq!(t.get("Booking", kim_id).await["resource"], json!(room_b.to_string()), "Room A is Ben's at 15:00");
    let full = t.book(massage_body("Late Comer", at(d, "15:00"), None)).await.unwrap_err();
    assert!(matches!(&full, Error::Conflict(m) if m.contains("no staff member is available")), "{full:?}");

    // ───────────── what guests may not do ─────────────
    let bookings_now = t.scalar("select count(*) from e_booking where tenant_id = $1 and $2::uuid is null", None).await;
    let bot = t.book(json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "name": "Bot", "email": "bot@example.test", "website": "http://spam.test"})).await.unwrap();
    assert_eq!(bot, json!({"status": "received"}), "the honeypot answers like a success");
    assert_eq!(t.scalar("select count(*) from e_booking where tenant_id = $1 and $2::uuid is null", None).await, bookings_now, "and books nothing");
    let refusals = [
        (json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00"))}), "name: is required | email: is required"),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "name": "N", "email": "n@example.test"}),
            "Any allergies?: an answer is required",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "maybe"}}),
            "answer Yes or No",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:10")), "staff": sid(&asha), "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "no"}}),
            "not one of the offered start times",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(Utc::now() + Duration::minutes(30)), "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "no"}}),
            "minimum notice",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(today + Duration::days(70), "10:00")), "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "no"}}),
            "too far ahead",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "13:00")), "staff": sid(&asha), "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "no"}}),
            "outside working hours",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "name": "N", "email": "n@example.test", "time_zone": "Mars/Base"}),
            "IANA",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "seats": 2, "name": "N", "email": "n@example.test", "answers": {"Any allergies?": "no"}}),
            "1 seat(s)",
        ),
        (
            json!({"service": yoga_id.to_string(), "start_time": rfc(at(d1, "09:00")), "staff": sid(&ben), "name": "N", "email": "n@example.test"}),
            "does not provide Morning Yoga",
        ),
        (
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "16:00")), "name": "N", "email": "not-an-email", "answers": {"Any allergies?": "no"}}),
            "valid email",
        ),
    ];
    for (body, why) in refusals {
        let e = t.book(body).await.unwrap_err();
        assert!(messages(&e).contains(why), "{why}: {}", messages(&e));
    }
    assert!(matches!(
        t.book(json!({"service": Uuid::now_v7().to_string(), "start_time": rfc(at(d, "16:00")), "name": "N", "email": "n@example.test"})).await,
        Err(Error::NotFound(_))
    ));

    // ───────────── racing for the same time ─────────────
    let race = |n: usize, staff: Option<String>, start: DateTime<Utc>, service: Uuid, prefix: &'static str| {
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let (k, site) = (t.k.clone(), t.site.clone());
                let body = json!({"service": service.to_string(), "start_time": rfc(start), "staff": staff, "name": format!("{prefix} {i}"),
                                  "email": format!("{prefix}{i}@example.test"), "answers": if service == massage_id { json!({"Any allergies?": "No"}) } else { json!({}) }});
                tokio::spawn(async move { site_call(&k, &site, tenant, "POST", "/book", Some(body), &format!("198.51.100.{i}")).await })
            })
            .collect();
        async move {
            let mut out = Vec::new();
            for h in handles {
                out.push(h.await.unwrap());
            }
            out
        }
    };
    let results = race(6, Some(sid(&asha)), at(d, "16:30"), massage_id, "racer").await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "exactly one of six gets Asha at 16:30");
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| matches!(e, Error::Conflict(_))), "the others are told the time is taken");
    let any_race = race(4, None, at(d1, "15:00"), massage_id, "anyrace").await;
    let winners: Vec<Value> = any_race.iter().filter_map(|r| r.as_ref().ok()).map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()).collect();
    assert_eq!(winners.len(), 2, "two staff, two rooms: two of four");
    let mut winning_staff: Vec<String> = winners.iter().map(|w| w["booking"]["staff"]["name"].as_str().unwrap().to_string()).collect();
    winning_staff.sort();
    assert_eq!(winning_staff, ["Asha Rao", "Ben Carter"]);
    let ben_racer = winners.iter().find(|w| w["booking"]["staff"]["name"] == json!("Ben Carter")).unwrap();
    let ben_racer_id = uid(&t.find_one("Booking", json!({"guest_email": ben_racer["booking"]["guest"]["email"]})).await);

    // The database refuses an overlapping row for the same staff member by itself...
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    let copy = |key: &str, shift: &str| {
        format!(
            "insert into e_booking (tenant_id, id, service, staff, status, start_time, end_time, block_start, block_end, session_key, seats) \
             select tenant_id, $2, service, staff, 'Confirmed', start_time + interval '{shift}', end_time + interval '{shift}', block_start + interval '{shift}', \
             block_end + interval '{shift}', {key}, 1 from e_booking where tenant_id = $1 and id = $3"
        )
    };
    let clash = sqlx::query(sqlx::AssertSqlSafe(copy("'intruder'", "30 minutes"))).bind(tenant).bind(Uuid::now_v7()).bind(uid(&b1)).execute(&mut *tx).await;
    assert_eq!(clash.unwrap_err().as_database_error().and_then(|e| e.code()).as_deref(), Some("23P01"), "exclusion constraint");
    tx.rollback().await.unwrap();
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    let unscheduled =
        sqlx::query("insert into e_booking (tenant_id, id, status) values ($1, $2, 'Confirmed')").bind(tenant).bind(Uuid::now_v7()).execute(&mut *tx).await;
    assert_eq!(unscheduled.unwrap_err().as_database_error().and_then(|e| e.code()).as_deref(), Some("23514"), "an active booking without a schedule");
    tx.rollback().await.unwrap();

    // ───────────── guests manage their bookings ─────────────
    let view = t.get_pub(&format!("/manage/{kim_token}")).await;
    assert_eq!(
        (view["booking"]["status"].clone(), view["booking"]["can_cancel"].clone(), view["booking"]["can_reschedule"].clone()),
        (json!("Confirmed"), json!(true), json!(true))
    );
    assert_eq!(view["booking"]["location"], json!("12 Lake Road, Pune"));
    assert_eq!(view["booking"]["cancel_until"], json!(rfc(at(d, "15:00") - Duration::hours(24))));
    for bad in ["/manage/not-a-token", &format!("/manage/{}", "A".repeat(43))] {
        assert!(matches!(t.site("GET", bad, None).await, Err(Error::NotFound(_))), "{bad}");
    }
    let own = t.get_pub(&format!("/manage/{kim_token}/slots?from={d}&to={d}&time_zone=Asia/Kolkata")).await;
    assert!(times_for(&own, &sid(&asha)).contains(&"15:00".to_string()), "the booking's own time does not block a move");
    let busy = t.post_pub(&format!("/manage/{kim_token}/reschedule"), json!({"start_time": rfc(at(d, "10:00"))})).await.unwrap_err();
    assert!(matches!(busy, Error::Conflict(_)), "{busy:?}");
    let to_ben = t.post_pub(&format!("/manage/{kim_token}/reschedule"), json!({"start_time": rfc(at(d, "15:00")), "staff": sid(&ben)})).await.unwrap_err();
    assert!(matches!(to_ben, Error::Conflict(_)), "Ben has Jane at 15:00: {to_ben:?}");
    let moved = t.post_pub(&format!("/manage/{kim_token}/reschedule"), json!({"start_time": rfc(at(d, "11:30"))})).await.unwrap();
    assert_eq!((moved["booking"]["start_time"].clone(), moved["booking"]["staff"]["name"].clone()), (json!(rfc(at(d, "11:30"))), json!("Asha Rao")));
    let kim_doc = t.get("Booking", kim_id).await;
    assert_eq!(
        (kim_doc["resource"].clone(), kim_doc["last_change"].clone(), kim_doc["notice_version"].clone()),
        (json!(room_a.to_string()), json!("Rescheduled"), json!(2))
    );
    assert!(times_for(&t.get_pub(&slots_path(massage_id, d, d, "Asia/Kolkata")).await, &sid(&asha)).contains(&"15:00".to_string()), "15:00 is free again");
    // Windows: guests cannot cancel or move inside them.
    t.update(&manager, "BookingSettings", settings_id, json!({"cancellation_window_hours": 8760, "reschedule_window_hours": 8760})).await;
    let late = t.post_pub(&format!("/manage/{jane_token}/cancel"), json!({})).await.unwrap_err();
    assert!(messages(&late).contains("can no longer be cancelled online"), "{late:?}");
    let late = t.post_pub(&format!("/manage/{jane_token}/reschedule"), json!({"start_time": rfc(at(d, "15:30"))})).await.unwrap_err();
    assert!(messages(&late).contains("can no longer be rescheduled online"), "{late:?}");
    t.update(&manager, "BookingSettings", settings_id, json!({"cancellation_window_hours": 24, "reschedule_window_hours": 12})).await;
    let cancelled = t.post_pub(&format!("/manage/{kim_token}/cancel"), json!({"reason": "Flight changed"})).await.unwrap();
    assert_eq!(
        (cancelled["booking"]["status"].clone(), cancelled["booking"]["can_cancel"].clone(), cancelled["booking"]["location"].clone()),
        (json!("Cancelled"), json!(false), Value::Null)
    );
    assert_eq!(t.get("Booking", kim_id).await["cancellation_reason"], json!("Flight changed"));
    assert!(messages(&t.post_pub(&format!("/manage/{kim_token}/cancel"), json!({})).await.unwrap_err()).contains("already Cancelled"));
    assert!(
        messages(&t.post_pub(&format!("/manage/{kim_token}/reschedule"), json!({"start_time": rfc(at(d, "15:00"))})).await.unwrap_err())
            .contains("only pending or confirmed")
    );

    // ───────────── a class: shared seats, a waitlist, promotion ─────────────
    let class = |name: &str, seats: i64, waitlist: bool, day: NaiveDate| json!({"service": yoga_id.to_string(), "start_time": rfc(at(day, "09:00")), "seats": seats, "name": name, "email": format!("{}@example.test", name.to_lowercase()), "waitlist": waitlist});
    let ana = t.book(class("Ana", 2, false, d1)).await.unwrap();
    let ana_token = ana["booking"].clone();
    assert_eq!(
        (ana_token["status"].clone(), ana_token["seats"].clone(), ana_token["location"].clone()),
        (json!("Confirmed"), json!(2), json!("https://meet.spa.test/yoga"))
    );
    let ana_tok = ana["token"].as_str().unwrap().to_string();
    let yoga_slots = t.get_pub(&slots_path(yoga_id, d1, d1, "Asia/Kolkata")).await;
    let nine = yoga_slots["days"][0]["slots"].as_array().unwrap().iter().find(|s| s["time"] == json!("09:00")).cloned().unwrap();
    assert_eq!(nine["seats_left"], json!(1));
    let bo = t.book(class("Bo", 1, false, d1)).await.unwrap();
    let (ana_doc, bo_doc) =
        (t.find_one("Booking", json!({"guest_email": "ana@example.test"})).await, t.find_one("Booking", json!({"guest_email": "bo@example.test"})).await);
    assert_eq!(ana_doc["session_key"], bo_doc["session_key"], "Bo joined Ana's session");
    assert_ne!(ana_doc["session_key"], ana_doc["id"]);
    assert_eq!(bo["booking"]["status"], json!("Confirmed"));
    let no_room = t.book(class("Cy", 1, false, d1)).await.unwrap_err();
    assert!(matches!(&no_room, Error::Conflict(m) if m.contains("the class is full") && m.contains("waitlist")), "{no_room:?}");
    let cy = t.book(class("Cy", 1, true, d1)).await.unwrap();
    assert_eq!(
        (cy["booking"]["status"].clone(), cy["booking"]["waitlist_position"].clone(), cy["booking"]["can_cancel"].clone()),
        (json!("Waitlisted"), json!(1), json!(true))
    );
    let di = t.book(class("Di", 2, true, d1)).await.unwrap();
    assert_eq!(di["booking"]["waitlist_position"], json!(2));
    // A full class is not offered as a time; asked for with waitlist=1 it is listed, marked full, so a page can offer its waitlist.
    assert!(
        !times_for(&t.get_pub(&slots_path(yoga_id, d1, d1, "Asia/Kolkata")).await, &sid(&asha)).contains(&"09:00".to_string()),
        "a full class is not offered"
    );
    let listed = t.get_pub(&format!("{}&waitlist=1", slots_path(yoga_id, d1, d1, "Asia/Kolkata"))).await;
    let nine = listed["days"][0]["slots"].as_array().unwrap().iter().find(|s| s["time"] == json!("09:00")).cloned().expect("the full class is listed");
    assert_eq!((nine["full"].clone(), nine["seats_left"].clone(), nine["staff"][0]["id"].clone()), (json!(true), json!(0), json!(sid(&asha))));
    let open = |v: &Value| v["days"].as_array().unwrap().iter().flat_map(|d| d["slots"].as_array().unwrap().iter()).any(|s| s["time"] == json!("09:00"));
    assert!(
        !open(&t.act(&manager, "BookingService", yoga_id, "find_slots", json!({"from": d1.to_string(), "to": d1.to_string()})).await),
        "staff book a full class through its waitlist, not as an open time"
    );
    // Ana leaves: Cy's one seat fits at once; Di's two do not yet.
    t.post_pub(&format!("/manage/{ana_tok}/cancel"), json!({})).await.unwrap();
    let cy_doc = t.find_one("Booking", json!({"guest_email": "cy@example.test"})).await;
    let di_doc = t.find_one("Booking", json!({"guest_email": "di@example.test"})).await;
    assert_eq!(
        (cy_doc["status"].clone(), cy_doc["last_change"].clone(), cy_doc["resource"].clone()),
        (json!("Confirmed"), json!("Promoted"), json!(studio.to_string()))
    );
    assert_eq!(di_doc["status"], json!("Waitlisted"));
    assert_eq!(t.get_pub(&format!("/manage/{}", di["token"].as_str().unwrap())).await["booking"]["waitlist_position"], json!(1));
    // Five guests race for the three seats of a fresh session: three get in.
    let seats_race = race(5, None, at(d2, "09:00"), yoga_id, "yogi").await;
    assert_eq!(seats_race.iter().filter(|r| r.is_ok()).count(), 3, "seats are counted under the staff lock");
    assert!(seats_race.iter().filter_map(|r| r.as_ref().err()).all(|e| matches!(e, Error::Conflict(m) if m.contains("full"))));

    // ───────────── notices and the in-app fallback (no Mail app yet) ─────────────
    t.update(&manager, "BookingService", yoga_id, json!({"capacity": 4})).await;
    t.maintain(Utc::now()).await;
    assert_eq!(
        t.find_one("Booking", json!({"guest_email": "di@example.test"})).await["status"],
        json!("Confirmed"),
        "the larger class seats Di in maintenance"
    );
    let ben_notes = t.notes(&ben_user).await;
    assert_eq!(ben_notes.len(), 2, "{ben_notes:?}");
    assert_eq!(ben_notes[0], "New booking: Deep Tissue Massage with Jane Doe");
    let asha_notes = t.notes(&asha_user).await;
    assert!(asha_notes.contains(&"New booking: Deep Tissue Massage with Meera Iyer".to_string()), "{asha_notes:?}");
    assert!(!asha_notes.iter().any(|n| n.contains("Kim Lee")), "made, moved and cancelled before anyone was told: nothing to say");
    assert!(!asha_notes.iter().any(|n| n.contains("Ana")), "{asha_notes:?}");
    let pending_notices = "select count(*) from e_booking where tenant_id = $1 and notice_version > notified_version and $2::uuid is null";
    assert_eq!(t.scalar(pending_notices, None).await, 1, "only Di's promotion, made after this round's notices");
    t.maintain(Utc::now()).await;
    assert_eq!(t.scalar(pending_notices, None).await, 0);
    assert!(t.notes(&asha_user).await.contains(&"Waitlist seat filled: Morning Yoga with Di".to_string()));
    let (claims, notes) = (
        t.scalar("select count(*) from bookings_notice where tenant_id = $1 and $2::uuid is null", None).await,
        t.scalar("select count(*) from notification where tenant_id = $1 and $2::uuid is null", None).await,
    );
    t.maintain(Utc::now()).await;
    assert_eq!(t.scalar("select count(*) from bookings_notice where tenant_id = $1 and $2::uuid is null", None).await, claims, "nothing is sent twice");
    assert_eq!(t.scalar("select count(*) from notification where tenant_id = $1 and $2::uuid is null", None).await, notes);
    assert_eq!(
        t.scalar("select count(*) from bookings_notice where tenant_id = $1 and channel = 'email' and $2::uuid is null", None).await,
        0,
        "no Mail app, no email"
    );
    // The fallback for guests: a link staff can copy.
    let link = t.act(&manager, "Booking", uid(&b1), "guest_link", json!({})).await;
    assert!(link["url"].as_str().unwrap().starts_with("https://spa.test/manage?token="));
    assert_eq!(t.get_pub(&format!("/manage/{}", link["token"].as_str().unwrap())).await["booking"]["guest"]["name"], json!("Meera Iyer"));

    // ───────────── with the Mail app: emails with invitations, approval ─────────────
    t.k.install_packages(tenant, &["comms".to_string()]).await.expect("install mail");
    let manager = t.reload(&manager).await;
    let asha_user = t.reload(&asha_user).await;
    t.update(&manager, "BookingSettings", settings_id, json!({"approval_required": true})).await;
    let liam = t
        .book(
            json!({"service": massage_id.to_string(), "start_time": rfc(at(d, "12:00")), "staff": sid(&asha), "name": "Liam Park", "email": "liam@example.test",
                     "answers": {"Any allergies?": "No"}}),
        )
        .await
        .unwrap();
    assert_eq!(liam["booking"]["status"], json!("Pending"), "public bookings wait for staff");
    let liam_id = uid(&t.find_one("Booking", json!({"guest_email": "liam@example.test"})).await);
    t.maintain(Utc::now()).await;
    let mails = t.mails_to("liam@example.test").await;
    assert_eq!(mails.len(), 1);
    let when_noon = at(d, "12:00").with_timezone(&ist).format("%a %-d %b %Y, %H:%M").to_string();
    assert_eq!(mails[0]["subject"], json!(format!("Booking request received: Deep Tissue Massage, {when_noon} (Asia/Kolkata)")));
    assert_eq!((mails[0]["reference_entity"].clone(), mails[0]["reference_id"].clone()), (json!("Booking"), json!(liam_id.to_string())));
    assert_eq!(mails[0]["ics"], Value::Null, "no invitation before it is confirmed");
    assert!(t.notes(&asha_user).await.contains(&"To confirm: Deep Tissue Massage with Liam Park".to_string()));
    let emailed = token_in(mails[0]["content"].as_str().unwrap());
    assert_eq!(t.get_pub(&format!("/manage/{emailed}")).await["booking"]["status"], json!("Pending"), "the emailed link works");
    let asha_note_count = t.notes(&asha_user).await.len();
    t.act(&asha_user, "Booking", liam_id, "confirm", json!({})).await;
    t.maintain(Utc::now()).await;
    let mails = t.mails_to("liam@example.test").await;
    assert_eq!(mails.len(), 2);
    assert!(mails[1]["subject"].as_str().unwrap().starts_with("Booking confirmed: Deep Tissue Massage"));
    let invite = mails[1]["ics"].as_str().unwrap();
    assert!(
        invite.contains("METHOD:REQUEST") && invite.contains("SEQUENCE:2") && invite.contains("ORGANIZER;CN=\"Asha Rao\":mailto:asha@spa.test"),
        "{invite}"
    );
    assert!(invite.contains(&format!("DTSTART:{}", at(d, "12:00").format("%Y%m%dT%H%M%SZ"))));
    assert_eq!(t.notes(&asha_user).await.len(), asha_note_count, "Asha confirmed it herself: no note");
    let moved = t.act(&manager, "Booking", liam_id, "reschedule", json!({"start_time": rfc(at(d, "11:30"))})).await;
    assert_eq!(moved["start_time"], json!(rfc(at(d, "11:30"))));
    t.maintain(Utc::now()).await;
    let mails = t.mails_to("liam@example.test").await;
    assert_eq!(mails.len(), 3);
    assert!(mails[2]["subject"].as_str().unwrap().starts_with("Booking moved: Deep Tissue Massage"));
    assert!(mails[2]["ics"].as_str().unwrap().contains("SEQUENCE:3"));
    assert!(t.notes(&asha_user).await.contains(&"Booking moved: Deep Tissue Massage with Liam Park".to_string()));
    t.maintain(Utc::now()).await;
    assert_eq!(t.mails_to("liam@example.test").await.len(), 3, "each change is told once");
    t.update(&manager, "BookingSettings", settings_id, json!({"approval_required": false})).await;

    // ───────────── reminders: each once, stale ones skipped ─────────────
    let start = at(d, "11:30");
    t.maintain(start - Duration::minutes(1440) + Duration::minutes(1)).await;
    t.maintain(start - Duration::minutes(1440) + Duration::minutes(2)).await;
    let mails = t.mails_to("liam@example.test").await;
    assert_eq!(mails.len(), 4);
    assert!(mails[3]["subject"].as_str().unwrap().starts_with("Reminder: Deep Tissue Massage"));
    t.maintain(start - Duration::minutes(30)).await;
    t.maintain(start - Duration::minutes(10)).await;
    assert_eq!(t.mails_to("liam@example.test").await.len(), 5, "a day before and an hour before, once each");
    let reminder_claims = "select count(*) from bookings_notice where tenant_id = $1 and booking_id = $2 and kind = 'reminder' and channel = 'email'";
    assert_eq!(t.scalar(reminder_claims, Some(liam_id)).await, 2);
    // Maintenance down for a day: only the nearest reminder goes out, the stale one is skipped.
    t.maintain(at(d1, "14:40")).await;
    let ben_racer_mail = ben_racer["booking"]["guest"]["email"].as_str().unwrap().to_string();
    let racer_mails: Vec<String> = t.mails_to(&ben_racer_mail).await.iter().map(|m| m["subject"].as_str().unwrap().to_string()).collect();
    assert_eq!(racer_mails.iter().filter(|s| s.starts_with("Reminder:")).count(), 1, "{racer_mails:?}");
    assert_eq!(
        t.texts(
            "select key || '=' || channel from bookings_notice where tenant_id = $1 and booking_id = $2 and kind = 'reminder' order by key",
            Some(ben_racer_id)
        )
        .await,
        [format!("1440@{}=skipped", at(d1, "15:00").timestamp()), format!("60@{}=email", at(d1, "15:00").timestamp())]
    );

    // ───────────── staff: own bookings, time off, the rules on edits ─────────────
    let mine = t.list(&asha_user, "Booking", json!([])).await.unwrap();
    assert!(!mine.is_empty() && mine.iter().all(|b| b["staff_email"] == json!("asha@spa.test")), "Asha sees her own bookings only");
    assert!(matches!(t.k.get(&asha_user, "Booking", jane_id, false).await, Err(Error::NotFound(_))));
    assert!(matches!(t.action(&asha_user, "Booking", jane_id, "complete", json!({})).await, Err(Error::NotFound(_))));
    assert!(matches!(t.list(&clerk, "Booking", json!([])).await, Err(Error::Forbidden(_))));
    let moved_by_hand = t.k.update(&asha_user, "Booking", uid(&b1), obj(json!({"start_time": rfc(at(d, "14:00"))})), None).await.unwrap_err();
    assert!(messages(&moved_by_hand).contains("Reschedule action"), "{moved_by_hand:?}");
    t.update(&asha_user, "Booking", uid(&b1), json!({"notes": "Prefers lavender oil"})).await;
    assert!(messages(&t.k.delete(&manager, "Booking", uid(&b1), None).await.unwrap_err()).contains("cancel an active booking"));
    let off = t
        .insert(&asha_user, "StaffTimeOff", json!({"staff": sid(&asha), "from_time": rfc(at(d, "16:00")), "to_time": rfc(at(d, "17:00")), "reason": "Dentist"}))
        .await;
    assert_eq!((off["staff_name"].clone(), off["conflicting_bookings"].clone()), (json!("Asha Rao"), json!(1)), "the 16:30 racer is inside it");
    let theirs =
        t.k.insert(&asha_user, "StaffTimeOff", obj(json!({"staff": sid(&ben), "from_time": rfc(at(d, "16:00")), "to_time": rfc(at(d, "17:00"))}))).await;
    assert!(matches!(theirs, Err(Error::Forbidden(_))), "{theirs:?}");
    assert!(t.list(&ben_user, "StaffTimeOff", json!([])).await.unwrap().is_empty(), "Ben does not see Asha's time off");
    assert_eq!(times_for(&t.get_pub(&slots_path(massage_id, d, d, "Asia/Kolkata")).await, &sid(&asha)), ["14:00", "14:30", "15:00"]);
    let ben_free = t.act(&ben_user, "BookingService", massage_id, "find_slots", json!({"from": d.to_string(), "to": d.to_string(), "staff": sid(&ben)})).await;
    let ben_times = times_for(&ben_free, &sid(&ben));
    assert!(!ben_times.is_empty() && !ben_times.contains(&"15:00".to_string()), "find_slots for agents and staff: {ben_times:?}");
    // A booking being moved sees its own time as free: Ben's 15:00 is Jane's.
    let own = t
        .act(
            &ben_user,
            "BookingService",
            massage_id,
            "find_slots",
            json!({"from": d.to_string(), "to": d.to_string(), "staff": sid(&ben), "booking": jane_id.to_string()}),
        )
        .await;
    assert!(times_for(&own, &sid(&ben)).contains(&"15:00".to_string()), "find_slots with the booking being moved");
    assert!(
        messages(&t.action(&ben_user, "BookingService", massage_id, "find_slots", json!({"booking": "nobody"})).await.unwrap_err())
            .contains("is not a booking id")
    );

    // ───────────── completion, no-shows, invoices ─────────────
    let yesterday = today - Duration::days(1);
    let priya =
        t.insert(&manager, "Booking", json!({"service": massage_id.to_string(), "staff": sid(&asha), "start_time": rfc(at(yesterday, "10:00")), "guest_name": "Priya Shah", "guest_email": "priya@example.test"})).await;
    let noah = t
        .insert(
            &manager,
            "Booking",
            json!({"service": massage_id.to_string(), "staff": sid(&asha), "start_time": rfc(at(yesterday, "11:30")), "guest_name": "Noah Fox"}),
        )
        .await;
    let early = t.action(&manager, "Booking", jane_id, "complete", json!({})).await.unwrap_err();
    assert!(messages(&early).contains("once it has started"), "{early:?}");
    assert_eq!(t.act(&asha_user, "Booking", uid(&priya), "complete", json!({})).await["status"], json!("Completed"));
    assert_eq!(t.act(&manager, "Booking", uid(&noah), "mark_no_show", json!({})).await["status"], json!("No Show"));
    assert!(matches!(t.action(&asha_user, "Booking", uid(&priya), "make_sales_invoice", json!({})).await, Err(Error::Forbidden(_))), "staff do not invoice");
    let inv = t.act(&manager, "Booking", uid(&priya), "make_sales_invoice", json!({})).await;
    assert_eq!(inv["entity"], json!("SalesInvoice"));
    let inv_id: Uuid = inv["id"].as_str().unwrap().parse().unwrap();
    let invoice = t.get("SalesInvoice", inv_id).await;
    let customer = t.get("Customer", invoice["customer"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!(
        (customer["customer_name"].clone(), customer["email"].clone()),
        (json!("Priya Shah"), json!("priya@example.test")),
        "the guest became a customer"
    );
    assert_eq!(t.get("Booking", uid(&priya)).await["customer"], customer["id"]);
    assert_eq!(
        (invoice["company"].clone(), invoice["items"][0]["qty"].clone(), invoice["items"][0]["rate"].clone(), invoice["net_total"].clone()),
        (json!(cid.to_string()), json!("1"), json!("1200"), json!("1200"))
    );
    assert_eq!(
        (invoice["total_taxes"].clone(), invoice["grand_total"].clone(), invoice["rounded_total"].clone()),
        (json!("216"), json!("1416"), json!("1416"))
    );
    let again = t.action(&manager, "Booking", uid(&priya), "make_sales_invoice", json!({})).await.unwrap_err();
    assert!(messages(&again).contains("already invoiced on a draft sales invoice"), "{again:?}");
    t.k.delete(&a, "SalesInvoice", inv_id, None).await.expect("drop the draft");
    assert_eq!(t.get("Booking", uid(&priya)).await["sales_invoice"], Value::Null, "deleting the draft frees the booking");
    let inv2: Uuid = t.act(&manager, "Booking", uid(&priya), "make_sales_invoice", json!({})).await["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(t.get("SalesInvoice", inv2).await["customer"], customer["id"], "the same customer, found by email");
    let submitted = t.k.submit(&a, "SalesInvoice", inv2, None).await.expect("submit invoice");
    let again = t.action(&manager, "Booking", uid(&priya), "make_sales_invoice", json!({})).await.unwrap_err();
    assert!(messages(&again).contains(&format!("already invoiced on {}", submitted["number"].as_str().unwrap())), "{again:?}");
    t.k.cancel(&a, "SalesInvoice", inv2, None).await.expect("cancel invoice");
    let inv3: Uuid = t.act(&manager, "Booking", uid(&priya), "make_sales_invoice", json!({})).await["id"].as_str().unwrap().parse().unwrap();
    assert_ne!(inv3, inv2, "a cancelled invoice lets the booking be billed again");
    let di_id = uid(&t.find_one("Booking", json!({"guest_email": "di@example.test"})).await);
    let class_inv =
        t.get("SalesInvoice", t.act(&manager, "Booking", di_id, "make_sales_invoice", json!({})).await["id"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!(
        (class_inv["items"][0]["qty"].clone(), class_inv["items"][0]["rate"].clone(), class_inv["net_total"].clone(), class_inv["grand_total"].clone()),
        (json!("2"), json!("500"), json!("1000"), json!("1180")),
        "two seats at the booked price"
    );
    let free = t.insert(&manager, "BookingService", json!({"service_name": "Consultation", "duration_minutes": 30, "staff": [{"staff": sid(&asha)}]})).await;
    let consult = t.insert(&manager, "Booking", json!({"service": sid(&free), "start_time": rfc(at(d2, "14:00")), "guest_name": "Omar"})).await;
    assert_eq!(consult["staff"], json!(sid(&asha)), "any staff: the only one");
    assert!(messages(&t.action(&manager, "Booking", uid(&consult), "make_sales_invoice", json!({})).await.unwrap_err()).contains("nothing to invoice"));
    let waiting = t
        .insert(
            &manager,
            "Booking",
            json!({"service": yoga_id.to_string(), "start_time": rfc(at(d2, "09:00")), "guest_name": "Wren", "seats": 2, "join_waitlist": true}),
        )
        .await;
    assert_eq!(waiting["status"], json!("Waitlisted"), "three of the four seats are taken: staff can queue a guest too");
    let ana_id = uid(&ana_doc);
    t.k.delete(&manager, "Booking", ana_id, None).await.expect("cancelled bookings may be deleted");
    let _ = ana_token;

    // ───────────── calendar feeds ─────────────
    let feed = t.act(&asha_user, "BookingStaff", asha_id, "calendar_feed", json!({})).await;
    let path = feed["path"].as_str().unwrap().to_string();
    assert!(feed["url"].as_str().unwrap().starts_with(&format!("https://erp.spa.test/api/site/spa/bookings/feeds/{asha_id}.ics?v=1&exp=")));
    assert_eq!(t.get("BookingSettings", settings_id).await["signing_secret"], json!(true), "the key now exists, write-only");
    let local_path = path.trim_start_matches("/api/site/spa/bookings").to_string();
    let cal = t.raw(&local_path).await.expect("feed");
    assert_eq!(cal.content_type, "text/calendar; charset=utf-8");
    let body = String::from_utf8(cal.body).unwrap().replace("\r\n ", "");
    assert!(body.contains("X-WR-CALNAME:Asha Rao — bookings"));
    assert!(body.contains("SUMMARY:Deep Tissue Massage: Meera Iyer"));
    assert!(body.contains("SUMMARY:Morning Yoga (4 booked)"), "one event per class session");
    assert!(!body.contains("Kim Lee") && !body.contains("Jane Doe"), "cancelled bookings and Ben's are not in Asha's feed");
    let tampered = local_path.replace("sig=", "sig=0");
    assert!(matches!(t.raw(&tampered).await, Err(Error::NotFound(_))));
    assert!(
        matches!(t.raw(&local_path.replace(&asha_id.to_string(), &ben_id.to_string())).await, Err(Error::NotFound(_))),
        "Asha's link opens nobody else's feed"
    );
    assert!(matches!(t.action(&ben_user, "BookingStaff", asha_id, "calendar_feed", json!({})).await, Err(Error::Forbidden(_))));
    let rotated = t.act(&asha_user, "BookingStaff", asha_id, "rotate_calendar_feed", json!({})).await;
    assert!(rotated["path"].as_str().unwrap().contains("?v=2&"));
    assert!(matches!(t.raw(&local_path).await, Err(Error::NotFound(_))), "the old link is retired");
    assert!(t.raw(rotated["path"].as_str().unwrap().trim_start_matches("/api/site/spa/bookings")).await.is_ok());
    let download = t.raw(&format!("/manage/{jane_token}/booking.ics")).await.expect("guest download");
    let ics = String::from_utf8(download.body).unwrap().replace("\r\n ", "");
    assert!(ics.contains("METHOD:PUBLISH") && ics.contains("SUMMARY:Deep Tissue Massage with Ben Carter") && ics.contains("STATUS:CONFIRMED"));
    assert!(download.headers.iter().any(|(k, v)| k == "content-disposition" && v.contains(".ics")));
    // Closing the page closes the guest endpoints; staff feeds keep working.
    t.update(&manager, "BookingSettings", settings_id, json!({"public_page": false})).await;
    assert!(matches!(t.site("GET", &format!("/manage/{jane_token}"), None).await, Err(Error::NotFound(_))));
    assert!(t.raw(rotated["path"].as_str().unwrap().trim_start_matches("/api/site/spa/bookings")).await.is_ok());

    // ───────────── reports ─────────────
    let day = [("from_date", d.to_string()), ("to_date", d.to_string())];
    let summary = t.report(&manager, "bookings_summary", &[day[0].clone(), day[1].clone(), ("group_by", "Status".into())]).await;
    let by_status: HashMap<String, Value> = summary["rows"].as_array().unwrap().iter().map(|r| (r["group"].as_str().unwrap().to_string(), r.clone())).collect();
    assert_eq!(
        (by_status["Confirmed"]["bookings"].clone(), by_status["Confirmed"]["seats"].clone(), by_status["Confirmed"]["amount"].clone()),
        (json!(4), json!(4), json!("4800"))
    );
    assert_eq!((by_status["Cancelled"]["bookings"].clone(), by_status["Cancelled"]["amount"].clone()), (json!(1), json!("0")));
    assert_eq!((summary["totals"]["bookings"].clone(), summary["totals"]["amount"].clone()), (json!(5), json!("4800")));
    let own = t.report(&asha_user, "bookings_summary", &[day[0].clone(), day[1].clone(), ("group_by", "Staff".into())]).await;
    assert_eq!(own["rows"].as_array().unwrap().iter().map(|r| r["group"].as_str().unwrap()).collect::<Vec<_>>(), ["Asha Rao"], "staff see their own numbers");
    assert_eq!(own["totals"]["bookings"], json!(4));
    let attendance = t.report(&manager, "bookings_attendance", &[day[0].clone(), day[1].clone(), ("group_by", "Staff".into())]).await;
    let asha_row = attendance["rows"].as_array().unwrap().iter().find(|r| r["group"] == json!("Asha Rao")).cloned().unwrap();
    assert_eq!((asha_row["bookings"].clone(), asha_row["cancelled"].clone(), asha_row["cancellation_rate"].clone()), (json!(4), json!(1), json!("25")));
    let past = t
        .report(&manager, "bookings_attendance", &[("from_date", yesterday.to_string()), ("to_date", yesterday.to_string()), ("group_by", "Service".into())])
        .await;
    assert_eq!(
        (past["totals"]["completed"].clone(), past["totals"]["no_show"].clone(), past["totals"]["no_show_rate"].clone()),
        (json!(1), json!(1), json!("50")),
        "one of the two who were due came"
    );
    let util = t.report(&manager, "bookings_staff_utilization", &day).await;
    let rows: HashMap<String, Value> = util["rows"].as_array().unwrap().iter().map(|r| (r["staff"].as_str().unwrap().to_string(), r.clone())).collect();
    assert_eq!(
        (rows["Asha Rao"]["available_minutes"].clone(), rows["Asha Rao"]["booked_minutes"].clone(), rows["Asha Rao"]["utilization"].clone()),
        (json!(420), json!(180), json!("42.86")),
        "eight hours less an hour off; Meera, Liam and the racer"
    );
    assert_eq!(
        (rows["Ben Carter"]["available_minutes"].clone(), rows["Ben Carter"]["booked_minutes"].clone(), rows["Ben Carter"]["utilization"].clone()),
        (json!(480), json!(60), json!("12.5"))
    );
    let own_util = t.report(&ben_user, "bookings_staff_utilization", &day).await;
    assert_eq!(own_util["rows"].as_array().unwrap().len(), 1);
    assert!(matches!(ferrum_domain::reports::run(&t.k, &clerk, "bookings_summary", &HashMap::new()).await, Err(Error::Forbidden(_))));

    // Staff edits follow their bookings (the email is what gives them access).
    t.update(&manager, "BookingStaff", ben_id, json!({"display_name": "Ben C.", "email": "ben.c@spa.test"})).await;
    let jane_now = t.get("Booking", jane_id).await;
    assert_eq!((jane_now["staff_name"].clone(), jane_now["staff_email"].clone()), (json!("Ben C."), json!("ben.c@spa.test")));

    // Guest links of long-past bookings are purged.
    let links = "select count(*) from bookings_token where tenant_id = $1 and booking_id = $2";
    assert!(t.scalar(links, Some(jane_id)).await >= 1);
    t.maintain(at(d, "16:00") + Duration::days(31)).await;
    assert_eq!(t.scalar(links, Some(jane_id)).await, 0);

    // ───────────── a series: the same local time each week, whatever the clocks do ─────────────
    let coaching = t
        .insert(
            &manager,
            "BookingService",
            json!({"service_name": "Coaching", "duration_minutes": 60, "location_type": "Video", "location_details": "https://meet.spa.test/c", "staff": [{"staff": ben_id.to_string()}]}),
        )
        .await;
    // The clocks go back in London on 27 October 2030, between the first date and the second.
    let on = |d: u32, m: u32| NaiveDate::from_ymd_opt(2030, m, d).unwrap();
    let session = |day: NaiveDate| json!({"service": sid(&coaching), "staff": ben_id.to_string(), "start_time": rfc(local(london, day, "10:00")), "guest_name": "Rae", "guest_email": "rae@spa.test"});
    let first = t.insert(&manager, "Booking", session(on(22, 10))).await;
    let first_id = uid(&first);
    let taken = t.insert(&manager, "Booking", session(on(29, 10))).await;
    let in_series = |series: &str| json!({"series": series});
    for bad in [json!({"every": "fortnightly", "count": 3}), json!({"every": "week", "count": 1}), json!({"every": "week", "count": 53}), json!({"count": 3})] {
        assert!(t.action(&manager, "Booking", first_id, "repeat", bad.clone()).await.is_err(), "{bad}");
    }
    // A series is whole or nothing: the week that is taken is named, and nothing was booked.
    let before = t.scalar("select count(*) from e_booking where tenant_id = $1 and guest_name = 'Rae'", None).await;
    let refused = t.action(&manager, "Booking", first_id, "repeat", json!({"every": "week", "count": 3})).await.unwrap_err();
    assert!(messages(&refused).contains("2030-10-29"), "{}", messages(&refused));
    assert_eq!(t.scalar("select count(*) from e_booking where tenant_id = $1 and guest_name = 'Rae'", None).await, before, "nothing was repeated");
    assert_eq!(t.get("Booking", first_id).await["series"], Value::Null);
    // Skipping what does not fit is the caller's choice, and the answer says what was skipped.
    // What the record page can give it asks for; what it cannot (a time, the free slots) is the
    // schedule's, and the page offers no button for it.
    let booking = manager.schema.entity("Booking").unwrap();
    let action = |name: &str| booking.def.actions.iter().find(|a| a.name == name).unwrap();
    for (name, asks) in [("repeat", &["every", "count", "skip_conflicts"][..]), ("cancel_booking", &["reason"]), ("cancel_series", &["reason"])] {
        assert_eq!(action(name).params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), asks, "{name}");
    }
    assert!(action("reschedule").hidden && manager.schema.entity("BookingService").unwrap().def.actions.iter().any(|a| a.name == "find_slots" && a.hidden));
    // The page's dialog says "Yes".
    let out = t.act(&manager, "Booking", first_id, "repeat", json!({"every": "week", "count": 4, "skip_conflicts": "Yes"})).await;
    assert_eq!(out["skipped"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["skipped"][0]["date"], json!("2030-10-29"));
    assert_eq!(out["booked"].as_array().unwrap().len(), 2, "{out}");
    let series = out["series"].as_str().unwrap().to_string();
    let rows = t.list(&a, "Booking", in_series(&series)).await.unwrap();
    assert_eq!(rows.len(), 3, "the first and two more");
    let mut starts: Vec<DateTime<Utc>> = rows.iter().map(|r| r["start_time"].as_str().unwrap().parse().unwrap()).collect();
    starts.sort();
    assert_eq!(starts, vec![local(london, on(22, 10), "10:00"), local(london, on(5, 11), "10:00"), local(london, on(12, 11), "10:00")]);
    assert_eq!(starts[0].format("%H:%M").to_string(), "09:00", "10:00 in summer time");
    assert_eq!(starts[1].format("%H:%M").to_string(), "10:00", "and 10:00 in winter time: an hour later in UTC, the same on the wall");
    assert!(rows.iter().all(|r| r["status"] == json!("Confirmed") && r["staff"] == json!(ben_id.to_string())));
    assert!(t.action(&manager, "Booking", first_id, "repeat", json!({"every": "week", "count": 2})).await.is_err(), "a booking joins one series");
    // The engine still places each one: the slot of a later date is gone for anyone else.
    let clash = t.k.insert(&manager, "Booking", obj(session(on(12, 11)))).await;
    assert!(clash.is_err(), "{clash:?}");
    // Each stays its own booking, and a series can end from any point on.
    let middle = rows.iter().find(|r| r["start_time"].as_str().unwrap().parse::<DateTime<Utc>>().unwrap() == starts[1]).unwrap();
    let ended = t.act(&manager, "Booking", uid(middle), "cancel_series", json!({"reason": "Moved away"})).await;
    assert_eq!(ended["cancelled"].as_array().unwrap().len(), 2, "this one and the later one: {ended}");
    assert_eq!(t.get("Booking", first_id).await["status"], json!("Confirmed"), "the earlier one stays");
    assert_eq!(t.get("Booking", uid(middle)).await["cancellation_reason"], json!("Moved away"));
    assert!(t.action(&manager, "Booking", uid(&taken), "cancel_series", json!({})).await.is_err(), "not in a series");

    // ───────────── a guest books a series from the page, where the service lets them ─────────────
    t.update(&manager, "BookingSettings", settings_id, json!({"public_page": true})).await;
    let week = |n: i64| today + Duration::days(7 * n);
    let weekly = |name: &str, day: NaiveDate, count: i64| {
        json!({"service": sid(&coaching), "staff": ben_id.to_string(), "start_time": rfc(local(london, day, "11:30")), "name": name,
               "email": format!("{}@example.test", name.to_lowercase()), "time_zone": "Europe/London", "repeat_every": "week", "repeat_count": count})
    };
    let once = t.book(weekly("Sol", week(5), 3)).await.unwrap_err();
    assert!(messages(&once).contains("one at a time"), "{}", messages(&once));
    assert!(t.get_pub("/services").await["services"].as_array().unwrap().iter().all(|s| s["repeats"] == json!(0)), "nothing offers a series yet");
    t.update(&manager, "BookingService", uid(&coaching), json!({"guest_repeat_max": 3})).await;
    let page = t.get_pub("/services").await;
    let offered = page["services"].as_array().unwrap().iter().find(|s| s["id"] == json!(sid(&coaching))).unwrap().clone();
    assert_eq!(offered["repeats"], json!(3), "{page}");
    assert!(messages(&t.book(weekly("Sol", week(5), 4)).await.unwrap_err()).contains("at most 3"));
    assert!(messages(&t.book(weekly("Sol", week(5), 1)).await.unwrap_err()).contains("2 or more"));
    let booked = t.book(weekly("Sol", week(5), 3)).await.unwrap();
    assert_eq!(booked["later"].as_array().unwrap().len(), 2, "{booked}");
    let sol = t.list(&a, "Booking", json!({"guest_email": "sol@example.test"})).await.unwrap();
    assert_eq!(sol.len(), 3, "this one and two more weeks");
    assert!(sol.iter().all(|r| r["series"] == sol[0]["series"] && !r["series"].is_null() && r["status"] == json!("Confirmed")));
    assert!(sol.iter().all(|r| r["source"] == json!("Public")), "each date was booked from the page");
    let mut weeks: Vec<DateTime<Utc>> = sol.iter().map(|r| r["start_time"].as_str().unwrap().parse().unwrap()).collect();
    weeks.sort();
    assert_eq!(weeks, vec![local(london, week(5), "11:30"), local(london, week(6), "11:30"), local(london, week(7), "11:30")]);
    // Whole or nothing for a guest too: the week that is taken is named, and nothing was booked.
    let clash = t.book(weekly("Tam", week(4), 2)).await.unwrap_err();
    assert!(messages(&clash).contains(&week(5).to_string()), "{}", messages(&clash));
    assert_eq!(t.scalar("select count(*) from e_booking where tenant_id = $1 and guest_email = 'tam@example.test'", None).await, 0);
    t.update(&manager, "BookingSettings", settings_id, json!({"public_page": false})).await;

    // ───────────── someone leaves: no more bookings, and those made stay for a person to move ─────────────
    let leaver = t.user("leaver@spa.test", &["Bookings User"]).await;
    let leaving = t
        .insert(
            &manager,
            "BookingStaff",
            json!({"display_name": "Leaver", "email": "Leaver@spa.test", "time_zone": "Europe/London", "working_hours": every("08:00", "16:00")}),
        )
        .await;
    let session_with = |who: &Doc, day: NaiveDate| json!({"service": sid(&coaching), "staff": sid(who), "start_time": rfc(local(london, day, "11:00")), "guest_name": "Una", "guest_email": "una@spa.test"});
    t.update(&manager, "BookingService", uid(&coaching), json!({"staff": [{"staff": ben_id.to_string()}, {"staff": sid(&leaving)}]})).await;
    let kept = t.insert(&manager, "Booking", session_with(&leaving, on(5, 11))).await;
    t.k.deprovision_user(&a, leaver.user, None, "left").await.expect("deprovisioned");
    while t.k.dispatch_outbox_once().await.unwrap() > 0 {}
    assert_eq!(t.get("BookingStaff", uid(&leaving)).await["active"], json!(false), "their calendar closes");
    assert!(t.k.insert(&manager, "Booking", obj(session_with(&leaving, on(6, 11)))).await.is_err(), "no more bookings");
    assert_eq!(t.get("Booking", uid(&kept)).await["status"], json!("Confirmed"), "what a guest was promised stays, for a person to move");

    // ───────────── her calendar elsewhere becomes busy at a booked time (FEO-032) ─────────────
    // Lou's booking is marked, Asha is told, and Lou's page says it may need another time. Moved, nothing
    // is left to resolve; busy at the new time too, it is marked again; kept, it is not marked again.
    t.update(&manager, "BookingSettings", settings_id, json!({"public_page": true})).await;
    let week_on = d + Duration::days(7);
    let open = times_for(&t.get_pub(&slots_path(massage_id, week_on, week_on, "Asia/Kolkata")).await, &sid(&asha));
    let (first, last) = (open.first().cloned().expect("Asha is free that day"), open.last().cloned().unwrap());
    let lou = t.book(massage_body("Lou Gray", at(week_on, &first), Some(&sid(&asha)))).await.unwrap();
    let lou_token = lou["token"].as_str().unwrap().to_string();
    let lou_id = uid(&t.find_one("Booking", json!({"guest_email": "lou@example.test"})).await);
    let busy_at = |hhmm: &str| {
        let busy = event("elsewhere", at(week_on, hhmm), at(week_on, hhmm) + Duration::hours(1), "");
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//elsewhere//EN\r\n{busy}END:VCALENDAR\r\n")
    };
    *calendar.lock().unwrap() = (200, busy_at(&first));
    t.update(&manager, "BookingStaff", asha_id, json!({"outside_calendar": url.clone()})).await;
    let later = Utc::now();
    t.maintain(later + Duration::hours(1)).await;
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(true), "marked");
    let told = "select count(*) from notification where tenant_id = $1 and user_id = $2 and title like 'Clashes with your calendar%'";
    assert_eq!(t.scalar(told, Some(asha_user.user)).await, 1, "Asha is told");
    let view = t.get_pub(&format!("/manage/{lou_token}")).await;
    assert_eq!((view["booking"]["clash"].clone(), view["booking"]["can_reschedule"].clone()), (json!(true), json!(true)), "{view}");
    // Free again elsewhere, the mark goes; busy again, it comes back.
    *calendar.lock().unwrap() = (200, "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//elsewhere//EN\r\nEND:VCALENDAR\r\n".to_string());
    t.maintain(later + Duration::minutes(80)).await;
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(false), "the calendar is free again");
    *calendar.lock().unwrap() = (200, busy_at(&first));
    t.maintain(later + Duration::minutes(100)).await;
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(true));
    assert_eq!(t.scalar(told, Some(asha_user.user)).await, 2);
    t.post_pub(&format!("/manage/{lou_token}/reschedule"), json!({"start_time": rfc(at(week_on, &last))})).await.unwrap();
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(false), "moved to a free time");
    assert_eq!(t.get_pub(&format!("/manage/{lou_token}")).await["booking"]["clash"], json!(false));
    *calendar.lock().unwrap() = (200, busy_at(&last));
    t.maintain(later + Duration::hours(2)).await;
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(true), "the new time clashes");
    assert_eq!(t.scalar(told, Some(asha_user.user)).await, 3);
    t.act(&manager, "Booking", lou_id, "keep_despite_clash", json!({})).await;
    let kept = t.get("Booking", lou_id).await;
    assert!(kept["outside_clash"] == json!(false) && kept["outside_clash_kept"].is_string(), "{kept:?}");
    t.maintain(later + Duration::hours(3)).await;
    assert_eq!(t.get("Booking", lou_id).await["outside_clash"], json!(false), "kept stays kept");
    assert_eq!(t.scalar(told, Some(asha_user.user)).await, 3, "and Asha is not told again");
    assert!(t.k.run_action(&manager, "Booking", lou_id, "keep_despite_clash", json!({})).await.is_err(), "nothing to keep");
    t.update(&manager, "BookingStaff", asha_id, json!({"outside_calendar": null})).await;
    t.update(&manager, "BookingSettings", settings_id, json!({"public_page": false})).await;

    // Writes are throttled per address and workspace, counted in the database for every replica.
    let cancel = |ip: &'static str| site_call(&t.k, &t.site, t.tenant, "POST", "/manage/no-such-link/cancel", Some(json!({})), ip);
    for _ in 0..30 {
        assert!(!matches!(cancel("10.7.0.1").await, Err(Error::TooManyRequests(_))));
    }
    assert!(matches!(cancel("10.7.0.1").await, Err(Error::TooManyRequests(_))), "the 31st write in ten minutes");
    assert!(!matches!(cancel("10.7.0.2").await, Err(Error::TooManyRequests(_))), "another address");
}

#[test]
fn deposits_end_to_end() {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(deposits()).await.unwrap() });
}

/// A facial of 1,000 takes a 30% deposit online, all of it back if cancelled two days ahead.
async fn deposits() {
    let url = std::env::var("FERRUM_BOOKINGS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_bookings_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 24, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    // Commerce's own packages (its Store Subscriptions connector needs Finance, which this test leaves out).
    packages.extend(ferrum_app_commerce::packages().expect("Commerce's packages").into_iter().filter(|p| p.name() != "commerce_subscriptions"));
    packages.extend(ferrum_app_bookings::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    ferrum_app_commerce::register(&mut hooks);
    ferrum_app_bookings::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    ferrum_app_erp::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "glow".into(),
            name: "Glow Studio".into(),
            admin_email: "admin@glow.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["bookings".into(), "commerce".into(), "bookings_payments".into()],
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Asia/Kolkata"})),
        })
        .await
        .expect("tenant");
    let session = k.login("glow", "admin@glow.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("bookings").cloned().expect("bookings site");
    let t = T { k: k.clone(), tenant, admin, site, calls: AtomicU32::new(0) };
    let a = t.admin.clone();
    let ist: Tz = "Asia/Kolkata".parse().unwrap();
    let day = Utc::now().with_timezone(&ist).date_naive() + Duration::days(3);
    let at = |hhmm: &str| local(ist, day, hhmm);

    // ───────────── a store to take the money, a studio to book ─────────────
    let inr = t.find_one("Currency", json!({"code": "INR"})).await;
    let cid = uid(&t.insert(&a, "Company", json!({"company_name": "Glow Studio", "abbr": "GS", "default_currency": inr["id"]})).await);
    let services_group = t.find_one("ItemGroup", json!({"group_name": "Services"})).await;
    let nos = t.find_one("Uom", json!({"uom_name": "Nos"})).await;
    let item = |code: &str| json!({"item_code": code, "item_name": code, "item_group": services_group["id"], "stock_uom": nos["id"], "is_stock_item": false});
    let facial_item = uid(&t.insert(&a, "Item", item("FACIAL")).await);
    let ship = uid(&t.insert(&a, "Item", item("SHIP")).await);
    let stores = t.find_one("Warehouse", json!({"warehouse_name": "Stores", "company": cid.to_string()})).await;
    let selling = t.find_one("PriceList", json!({"price_list_name": "Standard Selling"})).await;
    t.insert(
        &a,
        "StoreSettings",
        json!({"store_name": "Glow", "company": cid.to_string(), "currency": inr["id"], "price_list": selling["id"], "fulfilment_warehouse": stores["id"],
               "shipping_item": ship.to_string(), "payment_methods": [{"gateway": "fakepay", "label": "Card", "expiry_minutes": 30}], "store_url": "https://glow.test"}),
    )
    .await;
    let fakepay = {
        let role = "plugin:fakepay".to_string();
        for entity in ["PaymentRequest", "PaymentException", "OrderRefund"] {
            let p = ferrum_meta::PermissionDef { entity: entity.into(), read: true, write: true, ..Default::default() };
            t.k.set_role_permission(&a, &role, &p).await.unwrap();
        }
        let u = t.k.create_user(&a, "fakepay@plugins.ferrum.invalid", "Plugin: fakepay", None, &[role], false).await.unwrap();
        t.k.load_ctx(tenant, u, None, "plugin-fakepay".into()).await.unwrap()
    };
    let manager = t.user("manager@glow.test", &["Bookings Manager"]).await;
    t.insert(
        &manager,
        "BookingSettings",
        json!({"company": cid.to_string(), "public_page": true, "min_notice_minutes": 0, "max_advance_days": 60, "slot_interval_minutes": 30,
               "cancellation_window_hours": 0, "reschedule_window_hours": 0, "bpay_payment_method": "fakepay"}),
    )
    .await;
    let every: Vec<Value> = WEEK.iter().map(|d| json!({"weekday": d, "start_time": "09:00", "end_time": "18:00"})).collect();
    let mira = t
        .insert(&manager, "BookingStaff", json!({"display_name": "Mira", "email": "mira@glow.test", "time_zone": "Asia/Kolkata", "working_hours": every}))
        .await;
    let facial = uid(&t
        .insert(
            &manager,
            "BookingService",
            json!({"service_name": "Facial", "duration_minutes": 60, "price": "1000", "item": facial_item.to_string(), "location_type": "In Person",
                   "staff": [{"staff": sid(&mira)}], "bpay_deposit_percent": "30", "bpay_refund_hours": 48, "bpay_keep_percent": "100"}),
        )
        .await);
    let book = |hhmm: &'static str, email: &'static str| {
        let t = &t;
        async move {
            let v = t.book(json!({"service": facial.to_string(), "start_time": rfc(at(hhmm)), "name": "Guest", "email": email})).await.expect("booked");
            let number = v["booking"]["number"].as_str().unwrap().to_string();
            (uid(&t.find_one("Booking", json!({"number": number})).await), v["booking"].clone())
        }
    };

    // ───────────── a guest books: the booking waits for its deposit ─────────────
    let (first, view) = book("10:00", "asha@example.test").await;
    assert_eq!((view["status"].clone(), view["deposit"]["status"].clone()), (json!("Pending"), json!("Initiated")));
    assert_eq!(dec_of(&view["deposit"]["amount"]), "300", "30% of 1,000");
    let b = t.get("Booking", first).await;
    let invoice = t.get("SalesInvoice", b["sales_invoice"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!((invoice["docstatus"].clone(), dec_of(&invoice["grand_total"])), (json!(1), "1000".to_string()), "invoiced at once");
    let request: Uuid = b["bpay_payment_request"].as_str().unwrap().parse().unwrap();
    t.act(&fakepay, "PaymentRequest", request, "mark_paid", json!({"amount": "300", "currency": "INR", "gateway_reference": "pi_1"})).await;
    assert_eq!(t.get("Booking", first).await["status"], json!("Confirmed"), "paying the deposit confirms it");
    assert_eq!(dec_of(&t.get("SalesInvoice", uid(&invoice)).await["outstanding_amount"]), "700", "the rest is paid on the day");

    // ───────────── cancelled three days ahead: all 300 back ─────────────
    t.act(&manager, "Booking", first, "cancel_booking", json!({"reason": "Unwell"})).await;
    let b = t.get("Booking", first).await;
    assert_eq!(dec_of(&b["bpay_kept"]), "0");
    let refund: Uuid = b["bpay_refund"].as_str().unwrap().parse().unwrap();
    let r = t.get("OrderRefund", refund).await;
    assert_eq!((r["kind"].clone(), dec_of(&r["amount"]), r["status"].clone()), (json!("Payment"), "300".to_string(), json!("Requested")));
    let note = t.get("SalesInvoice", r["credit_note"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!(
        (dec_of(&note["grand_total"]), dec_of(&t.get("SalesInvoice", uid(&invoice)).await["outstanding_amount"])),
        ("-1000".to_string(), "0".to_string())
    );
    t.act(&a, "OrderRefund", refund, "approve_refund", json!({})).await;
    t.act(&fakepay, "OrderRefund", refund, "refund_done", json!({"refund_reference": "re_1"})).await;
    let r = t.get("OrderRefund", refund).await;
    assert_eq!((r["status"].clone(), r["booked"].clone()), (json!("Refunded"), json!(true)), "{}", r["booking_error"]);

    // ───────────── cancelled within the two days: the deposit is kept ─────────────
    t.update(&manager, "BookingService", facial, json!({"bpay_refund_hours": 100})).await;
    let (second, _) = book("12:00", "ben@example.test").await;
    let b = t.get("Booking", second).await;
    let request: Uuid = b["bpay_payment_request"].as_str().unwrap().parse().unwrap();
    t.act(&fakepay, "PaymentRequest", request, "mark_paid", json!({"amount": "300", "currency": "INR", "gateway_reference": "pi_2"})).await;
    t.act(&manager, "Booking", second, "cancel_booking", json!({"reason": "Changed plans"})).await;
    let b = t.get("Booking", second).await;
    assert_eq!((dec_of(&b["bpay_kept"]), b["bpay_refund"].clone()), ("300".to_string(), Value::Null), "nothing goes back");
    let invoice = t.get("SalesInvoice", b["sales_invoice"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!(dec_of(&invoice["outstanding_amount"]), "0", "credited for the 700 not kept");

    // ───────────── cancelled before paying: nothing was taken ─────────────
    let (third, _) = book("14:00", "omar@example.test").await;
    let b = t.get("Booking", third).await;
    t.act(&manager, "Booking", third, "cancel_booking", json!({})).await;
    let request = t.get("PaymentRequest", b["bpay_payment_request"].as_str().unwrap().parse().unwrap()).await;
    let invoice = t.get("SalesInvoice", b["sales_invoice"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!((request["status"].clone(), invoice["docstatus"].clone()), (json!("Cancelled"), json!(2)));
}

/// A decimal as text without trailing zeros.
fn dec_of(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    text.parse::<rust_decimal::Decimal>().map(|d| d.normalize().to_string()).unwrap_or(text)
}

/// A fresh database with one workspace (zone Asia/Kolkata) on Bookings and the packages in `extra`
/// (`commerce` and `bookings_payments` for deposits, `comms` for the Mail stand-in).
async fn boot(slug: &str, extra: &[&str]) -> T {
    let url = std::env::var("FERRUM_BOOKINGS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_bookings_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 24, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let commerce = extra.contains(&"commerce");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    if commerce {
        packages.extend(ferrum_app_commerce::packages().expect("Commerce's packages").into_iter().filter(|p| p.name() != "commerce_subscriptions"));
    }
    packages.extend(ferrum_app_bookings::packages().expect("app packages").into_iter().filter(|p| commerce || p.name() != "bookings_payments"));
    packages.push(mail_stand_in());
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    if commerce {
        ferrum_app_commerce::register(&mut hooks);
    }
    ferrum_app_bookings::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    ferrum_app_erp::maintenance(&k).await.expect("partitions");
    let mut installed = vec!["bookings".to_string()];
    installed.extend(extra.iter().map(|p| p.to_string()));
    let tenant = k
        .create_tenant(NewTenant {
            slug: slug.into(),
            name: slug.into(),
            admin_email: format!("admin@{slug}.test"),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: installed,
            settings: Some(json!({"fiscal_year_start_month": 4, "timezone": "Asia/Kolkata"})),
        })
        .await
        .expect("tenant");
    let session = k.login(slug, &format!("admin@{slug}.test"), "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let site = k.hooks.site("bookings").cloned().expect("bookings site");
    T { k: k.clone(), tenant, admin, site, calls: AtomicU32::new(0) }
}

fn run(f: impl std::future::Future<Output = ()> + Send + 'static) {
    let _db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(f).await.unwrap() });
}

/// The first `weekday` on or after `from`.
fn next(from: NaiveDate, weekday: chrono::Weekday) -> NaiveDate {
    use chrono::Datelike;
    from.iter_days().find(|d| d.weekday() == weekday).unwrap()
}

#[test]
fn working_calendars_end_to_end() {
    run(working_calendars());
}

/// A clinic's staff follow the workspace's "pune" calendar (weekdays 09:00-12:00): its holiday
/// closes a Monday for every service, a dated row opens a Saturday morning once, and a booking on a
/// day closed after it was made is kept and flagged.
async fn working_calendars() {
    let t = boot("clinic", &[]).await;
    let a = t.admin.clone();
    let ist: Tz = "Asia/Kolkata".parse().unwrap();
    let monday = next(Utc::now().with_timezone(&ist).date_naive() + Duration::days(3), chrono::Weekday::Mon);
    let (tuesday, saturday) = (monday + Duration::days(1), monday + Duration::days(5));
    {
        let mut tx = t.k.db.tenant_tx(t.tenant).await.unwrap();
        let hours = json!({"mon": [["09:00", "12:00"]], "tue": [["09:00", "12:00"]], "wed": [["09:00", "12:00"]], "thu": [["09:00", "12:00"]], "fri": [["09:00", "12:00"]]});
        sqlx::query("insert into logic_calendar (tenant_id, name, zone, weekends, holidays, hours) values ($1, 'pune', 'Asia/Kolkata', '{sat,sun}', $2, $3)")
            .bind(t.tenant)
            .bind(vec![monday])
            .bind(hours)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let settings =
        uid(&t.insert(&a, "BookingSettings", json!({"public_page": true, "min_notice_minutes": 0, "max_advance_days": 60, "slot_interval_minutes": 60})).await);
    let e = t.k.insert(&a, "BookingStaff", obj(json!({"display_name": "X", "email": "x@clinic.test", "calendar": "nowhere"}))).await.unwrap_err();
    assert!(messages(&e).contains("working calendar"), "{}", messages(&e));
    let staff = t.insert(&a, "BookingStaff", json!({"display_name": "Dr Rao", "email": "rao@clinic.test", "calendar": "pune"})).await;
    let svc = uid(&t.insert(&a, "BookingService", json!({"service_name": "Checkup", "duration_minutes": 60, "staff": [{"staff": sid(&staff)}]})).await);
    let who = sid(&staff);
    let day = |d: NaiveDate| {
        let (t, who) = (&t, &who);
        async move { times_for(&t.get_pub(&format!("/slots?service={svc}&from={d}&to={d}&time_zone=Asia/Kolkata")).await, who) }
    };
    assert!(day(monday).await.is_empty(), "the calendar's holiday removes that day's slots");
    assert_eq!(day(tuesday).await, ["09:00", "10:00", "11:00"], "no working hours of their own: the calendar's");
    assert!(day(saturday).await.is_empty(), "weekend");
    t.update(
        &a,
        "BookingSettings",
        settings,
        json!({"calendar_dates": [{"calendar": "pune", "date": saturday.to_string(), "change": "Open", "start_time": "10:00", "end_time": "12:00"}]}),
    )
    .await;
    assert_eq!(day(saturday).await, ["10:00", "11:00"], "a one-off open Saturday adds them");

    // Booked on Tuesday; then the business closes that Tuesday: the booking stays, flagged.
    let b =
        t.book(json!({"service": svc.to_string(), "start_time": rfc(local(ist, tuesday, "10:00")), "name": "Guest", "email": "g@example.test"})).await.unwrap();
    let bid = uid(&t.find_one("Booking", json!({"number": b["booking"]["number"]})).await);
    t.maintain(Utc::now()).await;
    assert_eq!(t.get("Booking", bid).await["closed_clash"], json!(false));
    t.update(
        &a,
        "BookingSettings",
        settings,
        json!({"calendar_dates": [{"calendar": "pune", "date": saturday.to_string(), "change": "Open", "start_time": "10:00", "end_time": "12:00"},
                                  {"date": tuesday.to_string(), "change": "Closed"}]}),
    )
    .await;
    assert!(day(tuesday).await.is_empty(), "a business-wide closure");
    t.maintain(Utc::now()).await;
    let kept = t.get("Booking", bid).await;
    assert_eq!((kept["status"].clone(), kept["closed_clash"].clone()), (json!("Confirmed"), json!(true)), "kept and flagged, not deleted");
    // A business calendar that does not exist is refused; one that does closes its holidays for everyone.
    let e = t.k.update(&a, "BookingSettings", settings, obj(json!({"calendar": "nowhere"})), None).await.unwrap_err();
    assert!(messages(&e).contains("working calendar"), "{}", messages(&e));
}

#[test]
fn service_rules_end_to_end() {
    run(service_rules());
}

/// The workspace asks 2 hours' notice, a day's cancellation window, no approval and a reminder a
/// day ahead; "Walk-in Consult" sets its own: no notice, approval, cancel until the start, a reminder
/// 30 minutes ahead. "Standard" sets nothing and follows the workspace.
async fn service_rules() {
    let t = boot("rules", &[]).await;
    let a = t.admin.clone();
    t.insert(
        &a,
        "BookingSettings",
        json!({"public_page": true, "min_notice_minutes": 120, "max_advance_days": 60, "slot_interval_minutes": 60,
               "cancellation_window_hours": 24, "reschedule_window_hours": 24, "reminders": [{"minutes_before": 1440}]}),
    )
    .await;
    let all_day: Vec<Value> = WEEK.iter().map(|d| json!({"weekday": d, "start_time": "00:00", "end_time": "24:00"})).collect();
    let s1 =
        t.insert(&a, "BookingStaff", json!({"display_name": "Lee", "email": "lee@rules.test", "time_zone": "Asia/Kolkata", "working_hours": all_day})).await;
    let s2 =
        t.insert(&a, "BookingStaff", json!({"display_name": "Kay", "email": "kay@rules.test", "time_zone": "Asia/Kolkata", "working_hours": all_day})).await;
    let walk_in = uid(&t
        .insert(
            &a,
            "BookingService",
            json!({"service_name": "Walk-in Consult", "duration_minutes": 30, "staff": [{"staff": sid(&s1)}], "min_notice_minutes": 0,
                   "approval_required": "Yes", "cancellation_window_hours": 0, "reminders": [{"minutes_before": 30}]}),
        )
        .await);
    let standard = uid(&t.insert(&a, "BookingService", json!({"service_name": "Standard", "duration_minutes": 30, "staff": [{"staff": sid(&s2)}]})).await);
    // The next whole hour at least an hour away: inside the workspace's notice, outside the walk-in's.
    let ist: Tz = "Asia/Kolkata".parse().unwrap();
    let soon = {
        let l = (Utc::now() + Duration::minutes(59)).with_timezone(&ist);
        local(ist, l.date_naive(), &format!("{:02}:00", chrono::Timelike::hour(&l))) + Duration::hours(1)
    };
    let book = |svc: Uuid, at: DateTime<Utc>| {
        let t = &t;
        async move { t.book(json!({"service": svc.to_string(), "start_time": rfc(at), "name": "Guest", "email": "g@example.test"})).await }
    };
    // Notice: the service's 0 wins; the other service's empty one falls back to the workspace's 2 hours.
    let e = book(standard, soon).await.unwrap_err();
    assert!(messages(&e).contains("minimum notice"), "{}", messages(&e));
    let mine = book(walk_in, soon).await.expect("no notice for walk-ins");
    // Approval: the service's Yes wins over the workspace's off; the other is confirmed at once.
    assert_eq!(mine["booking"]["status"], json!("Pending"));
    let theirs = book(standard, soon + Duration::hours(2)).await.unwrap();
    assert_eq!(theirs["booking"]["status"], json!("Confirmed"));
    // Cancellation window: until the start for the walk-in; a day for the other, so not online now.
    assert_eq!((mine["booking"]["can_cancel"].clone(), theirs["booking"]["can_cancel"].clone()), (json!(true), json!(false)));
    let e = t.post_pub(&format!("/manage/{}/cancel", theirs["token"].as_str().unwrap()), json!({})).await.unwrap_err();
    assert!(messages(&e).contains("24 hours"), "{}", messages(&e));
    // Reminders: 30 minutes before the walk-in (confirmed now); none for the other inside its day-ahead
    // reminder (booked after it was due).
    let id = uid(&t.find_one("Booking", json!({"number": mine["booking"]["number"]})).await);
    t.act(&a, "Booking", id, "confirm", json!({})).await;
    t.maintain(soon - Duration::minutes(29)).await;
    let keys = t.texts("select key from bookings_notice where tenant_id = $1 and booking_id = $2 and kind = 'reminder'", Some(id)).await;
    assert_eq!(keys, [format!("30@{}", soon.timestamp())], "the service's offset");
    t.post_pub(&format!("/manage/{}/cancel", mine["token"].as_str().unwrap()), json!({})).await.expect("cancelled online until the start");
}

#[test]
fn unpaid_deposits_end_to_end() {
    run(unpaid_deposits());
}

/// A peel of 1,000 takes a 30% deposit and holds its time 15 minutes for it (the settings leave the
/// hold empty: a facial holds until its deposit request expires, after 30 minutes).
async fn unpaid_deposits() {
    let t = boot("peel", &["commerce", "bookings_payments", "comms"]).await;
    let a = t.admin.clone();
    let ist: Tz = "Asia/Kolkata".parse().unwrap();
    let day = Utc::now().with_timezone(&ist).date_naive() + Duration::days(3);
    let at = |hhmm: &str| local(ist, day, hhmm);
    let inr = t.find_one("Currency", json!({"code": "INR"})).await;
    let cid = uid(&t.insert(&a, "Company", json!({"company_name": "Peel", "abbr": "PL", "default_currency": inr["id"]})).await);
    let group = t.find_one("ItemGroup", json!({"group_name": "Services"})).await;
    let nos = t.find_one("Uom", json!({"uom_name": "Nos"})).await;
    let item = |code: &str| json!({"item_code": code, "item_name": code, "item_group": group["id"], "stock_uom": nos["id"], "is_stock_item": false});
    let peel_item = uid(&t.insert(&a, "Item", item("PEEL")).await);
    let ship = uid(&t.insert(&a, "Item", item("SHIP")).await);
    let stores = t.find_one("Warehouse", json!({"warehouse_name": "Stores", "company": cid.to_string()})).await;
    let selling = t.find_one("PriceList", json!({"price_list_name": "Standard Selling"})).await;
    t.insert(
        &a,
        "StoreSettings",
        json!({"store_name": "Peel", "company": cid.to_string(), "currency": inr["id"], "price_list": selling["id"], "fulfilment_warehouse": stores["id"],
               "shipping_item": ship.to_string(), "payment_methods": [{"gateway": "fakepay", "label": "Card", "expiry_minutes": 30}], "store_url": "https://peel.test"}),
    )
    .await;
    let fakepay = {
        let role = "plugin:fakepay".to_string();
        for entity in ["PaymentRequest", "PaymentException", "OrderRefund"] {
            let p = ferrum_meta::PermissionDef { entity: entity.into(), read: true, write: true, ..Default::default() };
            t.k.set_role_permission(&a, &role, &p).await.unwrap();
        }
        let u = t.k.create_user(&a, "fakepay@plugins.ferrum.invalid", "Plugin: fakepay", None, &[role], false).await.unwrap();
        t.k.load_ctx(t.tenant, u, None, "plugin-fakepay".into()).await.unwrap()
    };
    t.insert(
        &a,
        "BookingSettings",
        json!({"company": cid.to_string(), "public_page": true, "min_notice_minutes": 0, "max_advance_days": 60, "slot_interval_minutes": 30,
               "cancellation_window_hours": 0, "reschedule_window_hours": 0, "bpay_payment_method": "fakepay"}),
    )
    .await;
    let every: Vec<Value> = WEEK.iter().map(|d| json!({"weekday": d, "start_time": "09:00", "end_time": "18:00"})).collect();
    let mira =
        t.insert(&a, "BookingStaff", json!({"display_name": "Mira", "email": "mira@peel.test", "time_zone": "Asia/Kolkata", "working_hours": every})).await;
    let service = |name: &str, hold: Option<i64>| {
        json!({"service_name": name, "duration_minutes": 60, "price": "1000", "item": peel_item.to_string(), "location_type": "In Person",
               "staff": [{"staff": sid(&mira)}], "bpay_deposit_percent": "30", "bpay_hold_minutes": hold})
    };
    let peel = uid(&t.insert(&a, "BookingService", service("Peel", Some(15))).await);
    let facial = uid(&t.insert(&a, "BookingService", service("Facial", None)).await);
    let book = |svc: Uuid, hhmm: &'static str, email: &'static str| {
        let t = &t;
        async move {
            let v = t.book(json!({"service": svc.to_string(), "start_time": rfc(at(hhmm)), "name": "Guest", "email": email})).await?;
            Ok::<Uuid, Error>(uid(&t.find_one("Booking", json!({"number": v["booking"]["number"]})).await))
        }
    };
    let pay = |b: Uuid| {
        let (t, fakepay) = (&t, &fakepay);
        async move {
            let pr: Uuid = t.get("Booking", b).await["bpay_payment_request"].as_str().unwrap().parse().unwrap();
            t.action(fakepay, "PaymentRequest", pr, "mark_paid", json!({"amount": "300", "currency": "INR", "gateway_reference": format!("pi_{b}")})).await
        }
    };
    let status = |b: Uuid| {
        let t = &t;
        async move { t.get("Booking", b).await["status"].as_str().unwrap().to_string() }
    };
    let subjects = |email: &'static str| {
        let t = &t;
        async move { t.mails_to(email).await.iter().map(|m| m["subject"].as_str().unwrap().to_string()).collect::<Vec<_>>() }
    };
    let soon = Utc::now() + Duration::minutes(16);

    // ───────────── held, then released after 15 minutes; paid late while free: restored ─────────────
    let ann = book(peel, "10:00", "ann@example.test").await.unwrap();
    assert_eq!(status(ann).await, "Pending");
    let e = book(peel, "10:00", "bob@example.test").await.unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "the unpaid deposit holds the time: {e:?}");
    t.maintain(Utc::now() + Duration::minutes(14)).await;
    assert_eq!(status(ann).await, "Pending", "inside its hold");
    t.maintain(soon).await;
    t.maintain(soon).await;
    assert_eq!(status(ann).await, "Expired");
    let mails = subjects("ann@example.test").await;
    assert_eq!(mails.iter().filter(|s| s.starts_with("Booking released")).count(), 1, "told once: {mails:?}");
    pay(ann).await.expect("still payable: its request lasts 30 minutes");
    assert_eq!(status(ann).await, "Confirmed", "the time was still free");
    t.maintain(soon).await;
    assert!(subjects("ann@example.test").await.last().unwrap().starts_with("Booking confirmed"));

    // ───────────── released and taken; paid late: refunded once ─────────────
    let cat = book(peel, "12:00", "cat@example.test").await.unwrap();
    t.maintain(soon).await;
    assert_eq!(status(cat).await, "Expired");
    let dan = book(peel, "12:00", "dan@example.test").await.expect("the released time");
    pay(cat).await.expect("paid late");
    assert_eq!((status(cat).await, status(dan).await), ("Expired".to_string(), "Pending".to_string()), "not restored over Dan");
    t.maintain(soon).await;
    t.maintain(soon).await;
    let b = t.get("Booking", cat).await;
    let refunds = t.list(&a, "OrderRefund", json!({"payment_request": b["bpay_payment_request"]})).await.unwrap();
    assert_eq!(refunds.len(), 1, "refunded exactly once");
    assert_eq!((dec_of(&refunds[0]["amount"]), dec_of(&b["bpay_kept"])), ("300".to_string(), "0".to_string()), "all of it");
    assert!(subjects("cat@example.test").await.last().unwrap().starts_with("Deposit refunded"));

    // ───────────── no hold set: the time is kept until the deposit request expires ─────────────
    let eve = book(facial, "14:00", "eve@example.test").await.unwrap();
    t.maintain(soon).await;
    assert_eq!(status(eve).await, "Pending", "the request is still payable");
    t.maintain(Utc::now() + Duration::minutes(31)).await;
    assert_eq!(status(eve).await, "Expired");
    let b = t.get("Booking", eve).await;
    let request = t.get("PaymentRequest", b["bpay_payment_request"].as_str().unwrap().parse().unwrap()).await;
    let invoice = t.get("SalesInvoice", b["sales_invoice"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!((request["status"].clone(), invoice["docstatus"].clone()), (json!("Cancelled"), json!(2)), "nothing more can be paid: both cancelled");

    // ───────────── a race: the hold running out, another guest, and the late payment at once ─────────────
    let fay = book(peel, "16:00", "fay@example.test").await.unwrap();
    let (_, gus, paid) = tokio::join!(t.maintain(soon), book(peel, "16:00", "gus@example.test"), pay(fay));
    t.maintain(soon).await;
    let active = t
        .scalar(
            &format!("select count(*) from e_booking where tenant_id = $1 and start_time = '{}' and status in ('Pending', 'Confirmed')", rfc(at("16:00"))),
            None,
        )
        .await;
    assert_eq!(active, 1, "one of them holds the time: gus {gus:?}, paid {paid:?}");
    let f = t.get("Booking", fay).await;
    if f["status"] == json!("Expired") && paid.is_ok() {
        assert_eq!(dec_of(&f["bpay_kept"]), "0", "Fay paid but lost the time: refunded");
    }
}
