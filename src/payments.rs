//! Deposits for paid bookings (the `bookings_payments` package, with Commerce). A guest's booking of
//! a service that takes a deposit waits as Pending: it is invoiced at once, and part of the invoice
//! is asked for through the store's payment method; paying it confirms the booking. Cancelling the
//! booking settles the deposit by the service's terms: cancelled early enough, all of it comes
//! back; later, the service keeps its share. The invoice is credited for what is not kept, and what
//! the guest paid beyond that goes back through the gateway, once a person approves the refund.

use crate::{doc_id, time};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use ferrum_domain::util::{d, elevated, get, id, money, rows, s, set, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Result, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) const PACKAGE: &str = "bookings_payments";

/// The share of the price a new booking takes as a deposit, and the method it is paid through:
/// for guests' bookings of paid services that take one, where a payment method is set.
async fn deposit_terms(tx: &mut Tx, tenant: Uuid, doc: &Doc) -> Result<Option<(Decimal, String)>> {
    if s(doc, "source") != Some("Public") || d(doc, "amount") <= Decimal::ZERO {
        return Ok(None);
    }
    let Some(service) = id(doc, "service") else { return Ok(None) };
    let percent: Option<Decimal> = sqlx::query_scalar("select bpay_deposit_percent from e_booking_service where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(service)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
    let method: Option<String> = sqlx::query_scalar("select bpay_payment_method from e_booking_settings where tenant_id = $1 limit 1")
        .bind(tenant)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
    Ok(match (percent.filter(|p| *p > Decimal::ZERO), method.filter(|m| !m.trim().is_empty())) {
        (Some(p), Some(m)) => Some((p, m)),
        _ => None,
    })
}

/// What the cancellation terms keep of a `deposit` cancelled at `now` for a booking starting at
/// `start`: nothing at least `refund_hours` before it, else `keep_percent` of it.
pub fn kept(deposit: Decimal, start: crate::Instant, now: crate::Instant, refund_hours: i64, keep_percent: Decimal) -> Decimal {
    if now <= start - Duration::hours(refund_hours) { Decimal::ZERO } else { (deposit * keep_percent / Decimal::ONE_HUNDRED).round_dp(2).min(deposit) }
}

pub struct BookingHook;

#[async_trait]
impl DocHook for BookingHook {
    /// A booking that takes a deposit waits for it (Bookings has chosen its first status by now).
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.contains_key("created_at") || s(doc, "status") != Some("Confirmed") {
            return Ok(());
        }
        if deposit_terms(hc.tx, hc.ctx.tenant, doc).await?.is_some() {
            set(doc, "status", "Pending");
            set(doc, "confirmed_at", Value::Null);
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        match old {
            None => ask_deposit(hc, doc).await,
            Some(old) if s(doc, "status") == Some("Cancelled") && s(old, "status") != Some("Cancelled") => settle(hc, doc).await,
            _ => Ok(()),
        }
    }
}

/// Invoices a new booking and asks for its deposit.
async fn ask_deposit(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let Some((percent, method)) = deposit_terms(hc.tx, hc.ctx.tenant, doc).await? else { return Ok(()) };
    if s(doc, "status") == Some("Waitlisted") {
        return Ok(());
    }
    let booking = doc_id(doc)?;
    let engine = elevated(hc.ctx);
    // The invoice a confirmed booking would get (the action waits for confirmation; this does not).
    crate::booking::invoice(hc, doc).await?;
    let fresh = hc.kernel.get_tx(&sys(hc.ctx), hc.tx, "Booking", booking, false).await?;
    let invoice = id(&fresh, "sales_invoice").ok_or_else(|| Error::internal("the booking was not invoiced"))?;
    let invoice = hc.kernel.submit_tx(&engine, hc.tx, "SalesInvoice", invoice, None).await?;
    let deposit = (d(&invoice, "grand_total") * percent / Decimal::ONE_HUNDRED).round_dp(2);
    let request = ferrum_app_commerce::request_invoice_payment(
        hc.kernel,
        &sys(hc.ctx),
        hc.tx,
        &invoice,
        &method,
        Some(deposit),
        s(doc, "guest_email").map(str::to_string),
    )
    .await?
    .ok_or_else(|| Error::internal("nothing to ask for on the booking's invoice"))?;
    let mut v = Doc::new();
    set(&mut v, "bpay_deposit", money(deposit));
    set(&mut v, "bpay_payment_request", s(&request, "id").unwrap_or_default());
    hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "Booking", booking, v).await
}

/// A cancelled booking's deposit, by the service's terms: an unpaid one stops being asked for (and
/// its invoice is cancelled); a paid one is kept in part, the invoice credited for the rest and
/// what was paid beyond what is kept asked to go back.
async fn settle(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let Some(pr) = id(doc, "bpay_payment_request") else { return Ok(()) };
    let sctx = sys(hc.ctx);
    let engine = elevated(hc.ctx);
    let booking = doc_id(doc)?;
    let request = hc.kernel.get_tx(&sctx, hc.tx, "PaymentRequest", pr, true).await?;
    let invoice = id(doc, "sales_invoice").ok_or_else(|| Error::internal("a booking with a deposit without its invoice"))?;
    if s(&request, "status") != Some("Paid") {
        if matches!(s(&request, "status"), Some("Initiated" | "Failed")) && request.get("docstatus").and_then(Value::as_i64) == Some(1) {
            hc.kernel.cancel_tx(&engine, hc.tx, "PaymentRequest", pr, None).await?;
        }
        let inv = hc.kernel.get_tx(&sctx, hc.tx, "SalesInvoice", invoice, false).await?;
        if inv.get("docstatus").and_then(Value::as_i64) == Some(1) && d(&inv, "outstanding_amount") == d(&inv, "grand_total") {
            hc.kernel.cancel_tx(&engine, hc.tx, "SalesInvoice", invoice, None).await?;
        }
        return Ok(());
    }
    let service = id(doc, "service").ok_or_else(|| Error::internal("booking without service"))?;
    let (hours, keep): (Option<i64>, Option<Decimal>) =
        sqlx::query_as("select bpay_refund_hours, bpay_keep_percent from e_booking_service where tenant_id = $1 and id = $2")
            .bind(hc.ctx.tenant)
            .bind(service)
            .fetch_one(&mut **hc.tx)
            .await?;
    let deposit = d(&request, "amount");
    let start = time(doc, "start_time").ok_or_else(|| Error::internal("booking without a start"))?;
    let keep = kept(deposit, start, Utc::now(), hours.unwrap_or(24), keep.unwrap_or(Decimal::ONE_HUNDRED));
    let inv = hc.kernel.get_tx(&sctx, hc.tx, "SalesInvoice", invoice, false).await?;
    let total = d(&inv, "grand_total");
    let mut v = Doc::new();
    set(&mut v, "bpay_kept", money(keep));
    if keep < total {
        let note = credit(hc, &engine, &inv, (total - keep) / total).await?;
        let owed_back = (-d(&note, "outstanding_amount")).max(Decimal::ZERO).min(deposit);
        if owed_back > Decimal::ZERO {
            let why =
                format!("Booking {} cancelled: {} of its deposit back by the service's terms", s(doc, "number").unwrap_or_default(), owed_back.normalize());
            let refund = ferrum_app_commerce::refund_payment(hc.kernel, &sctx, hc.tx, pr, owed_back, id(&note, "id"), &why).await?;
            set(&mut v, "bpay_refund", s(&refund, "id").unwrap_or_default());
        }
    }
    hc.kernel.set_values_tx(&sctx, hc.tx, "Booking", booking, v).await
}

/// A submitted credit note against `inv` for `share` of it: its lines at that share of their price.
async fn credit(hc: &mut HookCtx<'_>, actor: &Ctx, inv: &Doc, share: Decimal) -> Result<Doc> {
    let mut input = Doc::new();
    for f in ["company", "customer", "currency", "conversion_rate", "price_list", "tax_template", "customer_address", "cost_center"] {
        if let Some(v) = inv.get(f).filter(|v| !v.is_null()) {
            input.insert(f.to_string(), v.clone());
        }
    }
    set(&mut input, "is_return", true);
    set(&mut input, "return_against", s(inv, "id").unwrap_or_default());
    set(&mut input, "remarks", format!("Booking cancelled: credit of {}", s(inv, "number").unwrap_or_default()));
    let lines: Vec<Value> = rows(inv, "items")
        .iter()
        .map(|l| {
            json!({"item": l.get("item"), "qty": money(-d(l, "qty")), "price_list_rate": money((d(l, "price_list_rate") * share).round_dp(2)),
                   "discount_percent": "0", "description": l.get("description")})
        })
        .collect();
    set(&mut input, "items", Value::Array(lines));
    let draft = hc.kernel.insert_tx(actor, hc.tx, "SalesInvoice", input).await?;
    let note = id(&draft, "id").ok_or_else(|| Error::internal("credit note without id"))?;
    hc.kernel.submit_tx(actor, hc.tx, "SalesInvoice", note, None).await?;
    get(hc.kernel, &sys(hc.ctx), hc.tx, "SalesInvoice", note).await
}

/// On payment entries: a booking's deposit paid confirms the booking.
pub struct PaidHook;

#[async_trait]
impl DocHook for PaidHook {
    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(pr) = id(doc, "commerce_payment_request") else { return Ok(()) };
        let booking: Option<Uuid> = sqlx::query_scalar("select id from e_booking where tenant_id = $1 and bpay_payment_request = $2 and status = 'Pending'")
            .bind(hc.ctx.tenant)
            .bind(pr)
            .fetch_optional(&mut **hc.tx)
            .await?;
        let Some(booking) = booking else { return Ok(()) };
        let b = hc.kernel.get_tx(&sys(hc.ctx), hc.tx, "Booking", booking, true).await?;
        crate::booking::confirm(hc.kernel, &elevated(hc.ctx), hc.tx, &b).await?;
        Ok(())
    }
}

/// What a guest sees of their booking's deposit: how much, and the way to pay it while it is due.
pub async fn guest_view(ctx: &Ctx, tx: &mut Tx, b: &Doc) -> Result<Value> {
    let Some(pr) = id(b, "bpay_payment_request") else { return Ok(Value::Null) };
    let row: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("select status, currency_code, payment_url from e_payment_request where tenant_id = $1 and id = $2")
            .bind(ctx.tenant)
            .bind(pr)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((status, currency, url)) = row else { return Ok(Value::Null) };
    Ok(json!({
        "amount": money(d(b, "bpay_deposit")), "currency": currency, "status": status,
        "payment_url": if status == "Initiated" { url } else { None }, "kept": b.get("bpay_kept"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::str::FromStr;

    #[test]
    fn the_terms_keep_their_share_only_late() {
        let n = |v: &str| Decimal::from_str(v).unwrap();
        let start = Utc.with_ymd_and_hms(2026, 11, 20, 10, 0, 0).unwrap();
        assert_eq!(kept(n("300"), start, start - Duration::hours(49), 48, n("100")), n("0"), "two days ahead: all back");
        assert_eq!(kept(n("300"), start, start - Duration::hours(48), 48, n("100")), n("0"), "on the dot is in time");
        assert_eq!(kept(n("300"), start, start - Duration::hours(47), 48, n("100")), n("300"));
        assert_eq!(kept(n("300"), start, start - Duration::hours(1), 48, n("50")), n("150"));
        assert_eq!(kept(n("100.01"), start, start, 1, n("50")), n("50.00"), "half of 100.01, to the cent");
    }
}
