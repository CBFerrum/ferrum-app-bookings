//! The public booking page's endpoints, `/api/site/{tenant}/bookings/...` (the same routes answer
//! signed-in users at `/api/app/bookings/...`). Guests choose a service, a time, seats and their
//! details — never prices, staff assignments or statuses: every request is validated here and
//! acts with the system context through the same hooks as staff bookings. Guests reach their own
//! booking with the unguessable token they got (only its hash is stored); staff calendar feeds use
//! HMAC-signed links. Unknown, expired and forbidden all answer the same 404.

use crate::booking::{self, ACTIVE};
use crate::calendar::{self, ServiceInfo};
use crate::notify::{self, booking_event};
use crate::settings::{self, Settings};
use crate::{doc_id, ics, int, obj, slots, time, token, ts, when};
use async_trait::async_trait;
use chrono::Utc;
use chrono_tz::Tz;
use ferrum_domain::util::{d, id, money, rows, s};
use ferrum_kernel::{Ctx, Doc, Error, FieldError, Kernel, Result, Site, SiteRequest, SiteResponse, Tx, with_retry};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration as StdDuration, Instant as StdInstant};
use uuid::Uuid;

const MAX_BODY: usize = 64 * 1024;
/// Booking, rescheduling and cancelling per address and workspace, per window.
const WRITES_PER_WINDOW: u32 = 30;
const WINDOW: StdDuration = StdDuration::from_secs(600);

pub struct BookingSite;

#[async_trait]
impl Site for BookingSite {
    fn max_body(&self) -> usize {
        MAX_BODY
    }

    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        let schema = match &req.user {
            Some(u) => u.schema.clone(),
            None => k.registry.schema(req.tenant).await?,
        };
        let mut ctx = Ctx::system(req.tenant, schema);
        ctx.request_id = format!("bookings-{}", Uuid::now_v7());
        let path: Vec<&str> = req.path.split('/').filter(|p| !p.is_empty()).collect();
        let (status, body) = match (req.method.as_str(), path.as_slice()) {
            ("GET", ["feeds", file]) => return feed(k, &ctx, &req, file).await,
            ("GET", ["manage", tok, "booking.ics"]) => return download(k, &ctx, tok).await,
            ("GET", ["services"]) => (200, services(k, &ctx).await?),
            ("GET", ["slots"]) => (200, open_slots(k, &ctx, &req).await?),
            ("POST", ["book"]) => {
                throttle(req.tenant, &req.client_ip)?;
                book(k, &ctx, &body_of(&req)?).await?
            }
            ("GET", ["manage", tok]) => (200, manage(k, &ctx, tok).await?),
            ("GET", ["manage", tok, "slots"]) => (200, manage_slots(k, &ctx, &req, tok).await?),
            ("POST", ["manage", tok, "cancel"]) => {
                throttle(req.tenant, &req.client_ip)?;
                let body = body_of(&req)?;
                (200, change(k, &ctx, tok, Change::Cancel(body.get("reason").and_then(Value::as_str).map(str::to_string))).await?)
            }
            ("POST", ["manage", tok, "reschedule"]) => {
                throttle(req.tenant, &req.client_ip)?;
                let m = booking::parse_move(&Value::Object(body_of(&req)?))?;
                (200, change(k, &ctx, tok, Change::Move(m)).await?)
            }
            _ => return Err(Error::NotFound("page".into())),
        };
        let mut resp = SiteResponse::json(status, &body);
        resp.headers.push(("cache-control".into(), "no-store".into()));
        Ok(resp)
    }
}

/// A fixed-window limit per caller address and workspace on the requests that write (the server
/// already limits every public request; booking slots deserve a tighter budget).
fn throttle(tenant: Uuid, ip: &str) -> Result<()> {
    static HITS: LazyLock<Mutex<HashMap<String, (StdInstant, u32)>>> = LazyLock::new(Default::default);
    let mut hits = HITS.lock().unwrap_or_else(|e| e.into_inner());
    if hits.len() > 50_000 {
        hits.retain(|_, (since, _)| since.elapsed() < WINDOW);
        if hits.len() > 50_000 {
            hits.clear();
        }
    }
    let entry = hits.entry(format!("{tenant}|{ip}")).or_insert((StdInstant::now(), 0));
    if entry.0.elapsed() >= WINDOW {
        *entry = (StdInstant::now(), 0);
    }
    entry.1 += 1;
    if entry.1 > WRITES_PER_WINDOW {
        return Err(Error::TooManyRequests("too many booking requests from this address; try again in a few minutes".into()));
    }
    Ok(())
}

fn body_of(req: &SiteRequest) -> Result<Map<String, Value>> {
    if req.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    if req.body.len() > MAX_BODY {
        return Err(Error::invalid("request body larger than 64 KB"));
    }
    match serde_json::from_slice::<Value>(&req.body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(Error::invalid("the request body must be a JSON object")),
    }
}

fn query<'a>(req: &'a SiteRequest, key: &str) -> Option<&'a str> {
    req.query.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// The settings, when the public page is open; 404 otherwise.
async fn open_page(tx: &mut Tx, tenant: Uuid) -> Result<Settings> {
    let st = settings::load(tx, tenant).await?;
    if st.public_page { Ok(st) } else { Err(Error::NotFound("page".into())) }
}

fn workspace_zone(ctx: &Ctx) -> Tz {
    calendar::zone(&ctx.schema.timezone).unwrap_or(Tz::UTC)
}

/// An active service with someone to provide it, or 404.
async fn bookable(tx: &mut Tx, tenant: Uuid, raw: Option<&str>) -> Result<ServiceInfo> {
    let sid: Uuid = raw.and_then(|v| v.parse().ok()).ok_or_else(|| Error::NotFound("service".into()))?;
    calendar::service(tx, tenant, sid).await?.filter(|s| s.active && !s.staff.is_empty()).ok_or_else(|| Error::NotFound("service".into()))
}

async fn currency(tx: &mut Tx, tenant: Uuid, company: Option<Uuid>) -> Result<Option<String>> {
    let Some(c) = company else { return Ok(None) };
    Ok(sqlx::query_scalar(
        "select cur.code from e_company c join e_currency cur on cur.tenant_id = c.tenant_id and cur.id = c.default_currency where c.tenant_id = $1 and c.id = $2",
    )
    .bind(tenant)
    .bind(c)
    .fetch_optional(&mut **tx)
    .await?
    .flatten())
}

/// `GET /services`: what can be booked, by whom, and what will be asked.
async fn services(k: &Kernel, ctx: &Ctx) -> Result<Value> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let st = open_page(&mut tx, ctx.tenant).await?;
    let all = calendar::active_services(&mut tx, ctx.tenant).await?;
    let mut ids: Vec<Uuid> = all.iter().flat_map(|s| s.staff.iter().copied()).collect();
    ids.sort();
    ids.dedup();
    let staff = calendar::staff(&mut tx, ctx.tenant, &ids).await?;
    let code = currency(&mut tx, ctx.tenant, st.company).await?;
    tx.commit().await?;
    let list: Vec<Value> = all
        .iter()
        .filter_map(|svc| {
            let people: Vec<Value> = svc
                .staff
                .iter()
                .filter_map(|sid| staff.iter().find(|s| s.id == *sid && s.active))
                .map(|s| json!({"id": s.id, "name": s.name, "bio": s.bio}))
                .collect();
            (!people.is_empty()).then(|| {
                json!({
                    "id": svc.id,
                    "name": svc.name,
                    "description": svc.description,
                    "duration_minutes": svc.duration,
                    "seats": svc.capacity,
                    "price": money(svc.price),
                    "location_type": svc.location_type,
                    // Video links and phone numbers go to booked guests only.
                    "location": if svc.location_type == "In Person" { svc.location.clone() } else { None },
                    "staff": people,
                    "questions": svc.questions.iter().map(|q| json!({"question": q.question, "type": q.answer_type, "options": q.options, "required": q.required})).collect::<Vec<_>>(),
                })
            })
        })
        .collect();
    Ok(json!({"time_zone": ctx.schema.timezone, "currency": code, "services": list}))
}

/// `GET /slots?service=&from=&to=&time_zone=&staff=&seats=`.
async fn open_slots(k: &Kernel, ctx: &Ctx, req: &SiteRequest) -> Result<Value> {
    let q = slots::parse(|key| query(req, key).map(str::to_string), workspace_zone(ctx))?;
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let st = open_page(&mut tx, ctx.tenant).await?;
    let svc = bookable(&mut tx, ctx.tenant, query(req, "service")).await?;
    let out = slots::find(&mut tx, ctx.tenant, &svc, &st, &q, st.public_rules(Utc::now()), None).await?;
    tx.commit().await?;
    Ok(out)
}

fn text(body: &Map<String, Value>, key: &str, max: usize, errors: &mut Vec<FieldError>) -> Option<String> {
    let v = match body.get(key) {
        None | Some(Value::Null) => return None,
        Some(Value::String(v)) => v.trim().to_string(),
        Some(_) => {
            errors.push(FieldError::new(key, "must be text"));
            return None;
        }
    };
    if v.chars().count() > max {
        errors.push(FieldError::new(key, format!("at most {max} characters")));
        return None;
    }
    Some(v).filter(|v| !v.is_empty())
}

/// Intake answers checked against the service's questions: required ones present, numbers,
/// yes/no and choices valid, nothing unknown. Rows in question order.
fn answers(svc: &ServiceInfo, raw: Option<&Value>) -> Result<Vec<Value>> {
    let mut given: Vec<(String, String)> = Vec::new();
    let as_text = |v: &Value| match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "Yes".into() } else { "No".into() }),
        _ => None,
    };
    match raw {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            for (q, a) in m {
                given.push((q.trim().to_string(), as_text(a).ok_or_else(|| Error::field("answers", format!("{q}: answer with text")))?));
            }
        }
        Some(Value::Array(list)) => {
            for row in list {
                let q = row.get("question").and_then(Value::as_str).ok_or_else(|| Error::field("answers", "each answer needs its question"))?;
                let a = row.get("answer").and_then(as_text).unwrap_or_default();
                given.push((q.trim().to_string(), a));
            }
        }
        Some(_) => return Err(Error::field("answers", "send {question: answer} or [{question, answer}]")),
    }
    let mut errors = Vec::new();
    for (q, _) in &given {
        if !svc.questions.iter().any(|x| x.question.eq_ignore_ascii_case(q)) {
            errors.push(FieldError::new("answers", format!("{q}: not a question of this service")));
        }
    }
    let mut out = Vec::new();
    for q in &svc.questions {
        let answer = given.iter().find(|(x, _)| x.eq_ignore_ascii_case(&q.question)).map(|(_, a)| a.clone()).filter(|a| !a.is_empty());
        let Some(a) = answer else {
            if q.required {
                errors.push(FieldError::new("answers", format!("{}: an answer is required", q.question)));
            }
            continue;
        };
        let checked = match q.answer_type.as_str() {
            "Number" => a.parse::<rust_decimal::Decimal>().map(|n| n.normalize().to_string()).map_err(|_| "must be a number"),
            "Yes/No" => match a.to_lowercase().as_str() {
                "yes" | "true" | "1" => Ok("Yes".to_string()),
                "no" | "false" | "0" => Ok("No".to_string()),
                _ => Err("answer Yes or No"),
            },
            "Choice" => q.options.iter().find(|o| o.eq_ignore_ascii_case(&a)).cloned().ok_or("choose one of the options"),
            "Long Text" if a.chars().count() > 2000 => Err("at most 2000 characters"),
            "Long Text" => Ok(a),
            _ if a.chars().count() > 500 => Err("at most 500 characters"),
            _ => Ok(a.replace(['\r', '\n'], " ")),
        };
        match checked {
            Ok(v) => out.push(json!({"question": q.question, "answer": v})),
            Err(m) => errors.push(FieldError::new("answers", format!("{}: {m}", q.question))),
        }
    }
    if errors.is_empty() { Ok(out) } else { Err(Error::Validation(errors)) }
}

/// `POST /book`: a guest's booking. Returns it with its self-service token.
async fn book(k: &Kernel, ctx: &Ctx, body: &Map<String, Value>) -> Result<(u16, Value)> {
    // A form field people never see: whatever fills it is a bot, answered like a person.
    if body.get("website").is_some_and(|v| !v.is_null() && v.as_str() != Some("")) {
        return Ok((201, json!({"status": "received"})));
    }
    let mut errors = Vec::new();
    let name = text(body, "name", 200, &mut errors);
    let email = text(body, "email", 254, &mut errors);
    let phone = text(body, "phone", 40, &mut errors);
    let notes = text(body, "notes", 2000, &mut errors);
    let zone = text(body, "time_zone", 64, &mut errors);
    let start = text(body, "start_time", 64, &mut errors);
    if name.is_none() {
        errors.push(FieldError::new("name", "is required"));
    }
    if email.is_none() {
        errors.push(FieldError::new("email", "is required"));
    }
    if start.is_none() {
        errors.push(FieldError::new("start_time", "is required"));
    }
    if zone.as_deref().is_some_and(|z| calendar::zone(z).is_none()) {
        errors.push(FieldError::new("time_zone", "is not an IANA time zone (e.g. Europe/Paris)"));
    }
    let staff = match body.get("staff") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) if v.trim().is_empty() || v.trim().eq_ignore_ascii_case("any") => None,
        Some(Value::String(v)) => match v.trim().parse::<Uuid>() {
            Ok(u) => Some(u),
            Err(_) => {
                errors.push(FieldError::new("staff", "is not a staff id"));
                None
            }
        },
        Some(_) => {
            errors.push(FieldError::new("staff", "is not a staff id"));
            None
        }
    };
    let seats = match body.get("seats") {
        None | Some(Value::Null) => 1,
        Some(v) => match v.as_i64().or_else(|| v.as_str().and_then(|x| x.trim().parse().ok())) {
            Some(n) if (1..=1000).contains(&n) => n,
            _ => {
                errors.push(FieldError::new("seats", "must be a whole number from 1"));
                1
            }
        },
    };
    if !errors.is_empty() {
        return Err(Error::Validation(errors));
    }
    let waitlist = body.get("waitlist").and_then(Value::as_bool).unwrap_or(false);
    let service = body.get("service").and_then(Value::as_str).map(str::to_string);
    let view = with_retry(|| async {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let st = open_page(&mut tx, ctx.tenant).await?;
        let svc = bookable(&mut tx, ctx.tenant, service.as_deref()).await?;
        let rows = answers(&svc, body.get("answers"))?;
        let input = obj(json!({
            "service": svc.id.to_string(),
            "start_time": start,
            "staff": staff.map(|u| u.to_string()),
            "seats": seats,
            "source": "Public",
            "join_waitlist": waitlist,
            "guest_name": name,
            "guest_email": email,
            "guest_phone": phone,
            "guest_time_zone": zone.clone().unwrap_or_else(|| ctx.schema.timezone.clone()),
            "notes": notes,
            "answers": rows,
        }));
        let doc = k.insert_tx(ctx, &mut tx, "Booking", input).await?;
        let token = token::mint(&mut tx, ctx.tenant, doc_id(&doc)?).await?;
        let root = settings::site_root(&mut tx, ctx.tenant).await?;
        let view = guest_view(ctx, &mut tx, &doc, &st, &root, &token).await?;
        tx.commit().await?;
        Ok(json!({"booking": view, "token": token}))
    })
    .await?;
    Ok((201, view))
}

/// The booking behind a guest token, or 404 (unknown, expired and closed pages look the same).
async fn by_token(k: &Kernel, ctx: &Ctx, tx: &mut Tx, tok: &str, lock: bool) -> Result<(Settings, Doc)> {
    let st = open_page(tx, ctx.tenant).await?;
    let bid = token::resolve(tx, ctx.tenant, tok).await?.ok_or_else(|| Error::NotFound("booking".into()))?;
    let b = k.get_tx(ctx, tx, "Booking", bid, lock).await?;
    Ok((st, b))
}

/// What a guest sees of their booking.
async fn guest_view(ctx: &Ctx, tx: &mut Tx, b: &Doc, st: &Settings, root: &str, tok: &str) -> Result<Value> {
    let tz = notify::guest_zone(b, ctx);
    let now = Utc::now();
    let status = s(b, "status").unwrap_or_default();
    let start = time(b, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
    let end = time(b, "end_time").ok_or_else(|| Error::internal("booking without an end"))?;
    let active = ACTIVE.contains(&status);
    let position: Option<i64> = if status == "Waitlisted" {
        Some(
            sqlx::query_scalar::<_, i64>(
                "select count(*) + 1 from e_booking w join e_booking me on me.tenant_id = w.tenant_id and me.id = $2 \
                 where w.tenant_id = $1 and w.session_key = me.session_key and w.status = 'Waitlisted' and (w.created_at, w.id) < (me.created_at, me.id)",
            )
            .bind(ctx.tenant)
            .bind(doc_id(b)?)
            .fetch_one(&mut **tx)
            .await?,
        )
    } else {
        None
    };
    let (cancel_until, move_until) = (start - st.cancel_window, start - st.reschedule_window);
    Ok(json!({
        "number": b.get("number"),
        "status": status,
        "service": {"id": b.get("service"), "name": b.get("service_name")},
        "staff": {"id": b.get("staff"), "name": b.get("staff_name")},
        "start_time": ts(start),
        "end_time": ts(end),
        "time_zone": tz.name(),
        "when": when(start, tz),
        "until": end.with_timezone(&tz).format("%H:%M").to_string(),
        "seats": int(b, "seats"),
        "location_type": b.get("location_type"),
        "location": if status == "Cancelled" { Value::Null } else { b.get("location").cloned().unwrap_or(Value::Null) },
        "guest": {"name": b.get("guest_name"), "email": b.get("guest_email"), "phone": b.get("guest_phone")},
        "answers": rows(b, "answers").iter().map(|r| json!({"question": r.get("question"), "answer": r.get("answer")})).collect::<Vec<_>>(),
        "amount": money(d(b, "amount")),
        "cancellation_reason": b.get("cancellation_reason"),
        "waitlist_position": position,
        "can_cancel": status == "Waitlisted" || (active && now <= cancel_until),
        "cancel_until": active.then(|| ts(cancel_until)),
        "can_reschedule": active && now <= move_until,
        "reschedule_until": active.then(|| ts(move_until)),
        "links": {"manage": st.manage_link(root, tok), "ics": st.absolute(&format!("{root}/manage/{tok}/booking.ics"))},
    }))
}

/// `GET /manage/{token}`.
async fn manage(k: &Kernel, ctx: &Ctx, tok: &str) -> Result<Value> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let (st, b) = by_token(k, ctx, &mut tx, tok, false).await?;
    let root = settings::site_root(&mut tx, ctx.tenant).await?;
    let view = guest_view(ctx, &mut tx, &b, &st, &root, tok).await?;
    tx.commit().await?;
    Ok(json!({"booking": view}))
}

/// `GET /manage/{token}/slots?from=&to=&time_zone=&staff=`: times to move to (the booking's own
/// time does not block them).
async fn manage_slots(k: &Kernel, ctx: &Ctx, req: &SiteRequest, tok: &str) -> Result<Value> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let (st, b) = by_token(k, ctx, &mut tx, tok, false).await?;
    let mut q = slots::parse(|key| query(req, key).map(str::to_string), notify::guest_zone(&b, ctx))?;
    q.seats = int(&b, "seats").max(1);
    let sid = id(&b, "service").ok_or_else(|| Error::NotFound("booking".into()))?;
    let svc = calendar::service(&mut tx, ctx.tenant, sid).await?.ok_or_else(|| Error::NotFound("booking".into()))?;
    let out = slots::find(&mut tx, ctx.tenant, &svc, &st, &q, st.public_rules(Utc::now()), Some(doc_id(&b)?)).await?;
    tx.commit().await?;
    Ok(out)
}

enum Change {
    Cancel(Option<String>),
    Move(booking::Move),
}

/// `POST /manage/{token}/cancel {reason?}` and `/reschedule {start_time, staff?, seats?}`.
async fn change(k: &Kernel, ctx: &Ctx, tok: &str, what: Change) -> Result<Value> {
    with_retry(|| async {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let (st, b) = by_token(k, ctx, &mut tx, tok, true).await?;
        let after = match &what {
            Change::Cancel(reason) => booking::cancel(k, ctx, &mut tx, &b, reason.as_deref(), true).await?,
            Change::Move(m) => booking::reschedule(k, ctx, &mut tx, &b, m, true).await?,
        };
        let root = settings::site_root(&mut tx, ctx.tenant).await?;
        let view = guest_view(ctx, &mut tx, &after, &st, &root, tok).await?;
        tx.commit().await?;
        Ok(json!({"booking": view}))
    })
    .await
}

fn calendar_response(body: String, filename: Option<&str>) -> SiteResponse {
    let mut r = SiteResponse::text(200, "text/calendar; charset=utf-8", body);
    if let Some(f) = filename {
        r.headers.push(("content-disposition".into(), format!("attachment; filename=\"{f}\"")));
    }
    r.headers.push(("cache-control".into(), "private, max-age=300".into()));
    r.headers.push(("x-content-type-options".into(), "nosniff".into()));
    r
}

/// `GET /manage/{token}/booking.ics`: the booking for the guest's calendar.
async fn download(k: &Kernel, ctx: &Ctx, tok: &str) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let (_, b) = by_token(k, ctx, &mut tx, tok, false).await?;
    tx.commit().await?;
    let status = match s(&b, "status") {
        Some("Cancelled") => "CANCELLED",
        Some("Confirmed" | "Completed") => "CONFIRMED",
        _ => "TENTATIVE",
    };
    let summary = format!("{} with {}", s(&b, "service_name").unwrap_or("Booking"), s(&b, "staff_name").unwrap_or("us"));
    let event = booking_event(&b, summary, status).ok_or_else(|| Error::NotFound("booking".into()))?;
    let name: String = s(&b, "number").unwrap_or("booking").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    Ok(calendar_response(ics::calendar(Some("PUBLISH"), None, &[event]), Some(&format!("{name}.ics"))))
}

type FeedRow = (
    Uuid,
    Option<String>,
    Option<String>,
    crate::Instant,
    crate::Instant,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// `GET /feeds/{staff}.ics?v=&exp=&sig=`: a staff member's bookings (30 days back, a year ahead),
/// class sessions as one event each. Works whether or not the public page is open.
async fn feed(k: &Kernel, ctx: &Ctx, req: &SiteRequest, file: &str) -> Result<SiteResponse> {
    let not_found = || Error::NotFound("calendar".into());
    let staff: Uuid = file.strip_suffix(".ics").and_then(|v| v.parse().ok()).ok_or_else(not_found)?;
    let version: i64 = query(req, "v").and_then(|v| v.parse().ok()).ok_or_else(not_found)?;
    let expires: i64 = query(req, "exp").and_then(|v| v.parse().ok()).ok_or_else(not_found)?;
    let sig = query(req, "sig").ok_or_else(not_found)?;
    if expires < Utc::now().timestamp() {
        return Err(not_found());
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let key = settings::existing_key(k, &mut tx, ctx.tenant).await?.ok_or_else(not_found)?;
    if !token::feed_signature_valid(&key, ctx.tenant, staff, version, expires, sig) {
        return Err(not_found());
    }
    let member = calendar::staff(&mut tx, ctx.tenant, &[staff]).await?.into_iter().next().filter(|m| m.feed_version == version).ok_or_else(not_found)?;
    let rows: Vec<FeedRow> = sqlx::query_as(
        "select id, number, service_name, start_time, end_time, session_key, seats, guest_name, guest_phone, status, location, notice_version from e_booking \
         where tenant_id = $1 and staff = $2 and status in ('Pending', 'Confirmed', 'Completed', 'No Show') \
         and end_time > now() - interval '30 days' and start_time < now() + interval '366 days' order by start_time, id limit 3000",
    )
    .bind(ctx.tenant)
    .bind(staff)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    // One event per session: a class with five guests is one hour in the teacher's calendar.
    let mut sessions: Vec<(String, Vec<FeedRow>)> = Vec::new();
    for r in rows {
        match sessions.iter_mut().find(|(k, _)| *k == r.5) {
            Some((_, list)) => list.push(r),
            None => sessions.push((r.5.clone(), vec![r])),
        }
    }
    let events: Vec<ics::Event> = sessions
        .into_iter()
        .map(|(key, list)| {
            let first = &list[0];
            let service = first.2.clone().unwrap_or_else(|| "Booking".into());
            let class = key != first.0.to_string();
            let guests: Vec<String> = list
                .iter()
                .map(|r| {
                    let mut g = format!("{} ({})", r.7.clone().unwrap_or_else(|| "Guest".into()), r.1.clone().unwrap_or_default());
                    if r.6.unwrap_or(1) > 1 {
                        g.push_str(&format!(", {} seats", r.6.unwrap_or(1)));
                    }
                    if let Some(p) = &r.8 {
                        g.push_str(&format!(", {p}"));
                    }
                    g
                })
                .collect();
            let seats: i64 = list.iter().map(|r| r.6.unwrap_or(1)).sum();
            let summary =
                if class { format!("{service} ({seats} booked)") } else { format!("{service}: {}", first.7.clone().unwrap_or_else(|| "Guest".into())) };
            ics::Event {
                uid: if class { format!("{}@bookings.ferrum", key.replace('/', "-")) } else { format!("{}@bookings.ferrum", first.0) },
                sequence: list.iter().map(|r| r.11.unwrap_or(0)).max().unwrap_or(0),
                stamp: Utc::now(),
                start: first.3,
                end: list.iter().map(|r| r.4).max().unwrap_or(first.4),
                summary,
                description: Some(guests.join("\n")),
                location: first.10.clone(),
                status: if list.iter().all(|r| r.9.as_deref() == Some("Pending")) { "TENTATIVE" } else { "CONFIRMED" },
                organizer: None,
                attendees: vec![],
            }
        })
        .collect();
    Ok(calendar_response(ics::calendar(None, Some(&format!("{} — bookings", member.name)), &events), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Question;

    fn svc(questions: Vec<Question>) -> ServiceInfo {
        ServiceInfo {
            id: Uuid::now_v7(),
            name: "Massage".into(),
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
            questions,
        }
    }

    fn question(q: &str, kind: &str, options: &[&str], required: bool) -> Question {
        Question { question: q.into(), answer_type: kind.into(), options: options.iter().map(|o| o.to_string()).collect(), required }
    }

    #[test]
    fn intake_answers_are_checked() {
        let s = svc(vec![
            question("Any allergies?", "Yes/No", &[], true),
            question("Pressure", "Choice", &["Soft", "Medium", "Firm"], false),
            question("Age", "Number", &[], false),
            question("Notes", "Long Text", &[], false),
        ]);
        let rows = answers(&s, Some(&json!({"any allergies?": true, "Pressure": "firm", "Age": "42.0"}))).unwrap();
        assert_eq!(
            rows,
            vec![
                json!({"question": "Any allergies?", "answer": "Yes"}),
                json!({"question": "Pressure", "answer": "Firm"}),
                json!({"question": "Age", "answer": "42"})
            ]
        );
        let rows = answers(&s, Some(&json!([{"question": "Any allergies?", "answer": "no"}, {"question": "Notes", "answer": "Left\nshoulder"}]))).unwrap();
        assert_eq!(rows[1], json!({"question": "Notes", "answer": "Left\nshoulder"}));
        let errors = |v: Value| match answers(&s, Some(&v)) {
            Err(Error::Validation(e)) => e.into_iter().map(|e| e.message).collect::<Vec<_>>(),
            other => panic!("expected validation errors, got {other:?}"),
        };
        assert_eq!(errors(json!({})), ["Any allergies?: an answer is required"]);
        assert_eq!(
            errors(json!({"Any allergies?": "maybe", "Pressure": "Hard", "Age": "old", "Shoe size": "9"})),
            ["Shoe size: not a question of this service", "Any allergies?: answer Yes or No", "Pressure: choose one of the options", "Age: must be a number"]
        );
        assert!(answers(&s, Some(&json!("yes"))).is_err());
        assert!(answers(&svc(vec![]), None).unwrap().is_empty());
    }

    #[test]
    fn writes_are_throttled_per_address() {
        let tenant = Uuid::now_v7();
        for _ in 0..WRITES_PER_WINDOW {
            throttle(tenant, "198.51.100.1").unwrap();
        }
        assert!(matches!(throttle(tenant, "198.51.100.1"), Err(Error::TooManyRequests(_))));
        assert!(throttle(tenant, "198.51.100.2").is_ok(), "another address");
        assert!(throttle(Uuid::now_v7(), "198.51.100.1").is_ok(), "another workspace");
    }
}
