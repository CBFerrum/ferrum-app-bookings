//! When someone leaves (`ferrum.user.deprovisioned`), they take no more bookings. Bookings they
//! already have stay with them, for a person to move: a guest was promised a particular person, and
//! nothing here can decide who else will do.

use async_trait::async_trait;
use ferrum_kernel::lifecycle::DEPROVISIONED_TOPIC;
use ferrum_kernel::outbox::Event;
use ferrum_kernel::{EventSink, Kernel, Result};
use serde_json::Value;

pub struct Departures;

#[async_trait]
impl EventSink for Departures {
    async fn on_event(&self, k: &Kernel, ev: &Event) -> Result<()> {
        if ev.topic != DEPROVISIONED_TOPIC || k.registry.schema(ev.tenant_id).await?.entity("BookingStaff").is_none() {
            return Ok(());
        }
        let email = ev.payload.get("email").and_then(Value::as_str).unwrap_or_default().trim().to_lowercase();
        let mut tx = k.db.tenant_tx(ev.tenant_id).await?;
        sqlx::query("update e_booking_staff set active = false where tenant_id = $1 and lower(email::text) = $2 and active")
            .bind(ev.tenant_id)
            .bind(&email)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
