//! Secrets handed to guests and staff.
//!
//! * Guest self-service tokens: 256 random bits; only their SHA-256 is stored (`bookings_token`).
//!   A booking may have several (the booking response and every email mint one), each valid until
//!   30 days after the booking ends.
//! * Staff calendar feed links: an HMAC-SHA256 over tenant, staff member, feed version and expiry,
//!   keyed by the Booking Settings' secret field and checked in constant time. Bumping the staff
//!   member's feed version retires their links; replacing the secret retires everyone's.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferrum_kernel::{Result, Tx};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// How long after a booking ends its guest links keep working.
pub const LINK_DAYS: i64 = 30;

/// A new token and the hash to store.
pub fn new_token() -> (String, String) {
    let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let hash = hash(&token);
    (token, hash)
}

pub fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Shaped like a token we issue (43 URL-safe base64 characters); nothing else is looked up.
pub fn well_formed(token: &str) -> bool {
    token.len() == 43 && token.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Issues a guest token for `booking` (in the caller's transaction).
pub async fn mint(tx: &mut Tx, tenant: Uuid, booking: Uuid) -> Result<String> {
    let (token, hash) = new_token();
    sqlx::query("insert into bookings_token (tenant_id, token_hash, booking_id) values ($1, $2, $3)")
        .bind(tenant)
        .bind(&hash)
        .bind(booking)
        .execute(&mut **tx)
        .await?;
    Ok(token)
}

/// The booking a guest token opens, if it is ours and still valid.
pub async fn resolve(tx: &mut Tx, tenant: Uuid, token: &str) -> Result<Option<Uuid>> {
    if !well_formed(token) {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "select t.booking_id from bookings_token t join e_booking b on b.tenant_id = t.tenant_id and b.id = t.booking_id \
         where t.tenant_id = $1 and t.token_hash = $2 and b.end_time + make_interval(days => $3) > now()",
    )
    .bind(tenant)
    .bind(hash(token))
    .bind(LINK_DAYS as i32)
    .fetch_optional(&mut **tx)
    .await?)
}

fn feed_mac(key: &[u8], tenant: Uuid, staff: Uuid, version: i64, expires: i64) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(format!("ferrum-bookings-feed:{tenant}:{staff}:{version}:{expires}").as_bytes());
    mac
}

pub fn feed_signature(key: &[u8], tenant: Uuid, staff: Uuid, version: i64, expires: i64) -> String {
    hex::encode(feed_mac(key, tenant, staff, version, expires).finalize().into_bytes())
}

/// Constant-time check of a [`feed_signature`].
pub fn feed_signature_valid(key: &[u8], tenant: Uuid, staff: Uuid, version: i64, expires: i64, signature: &str) -> bool {
    let Ok(sig) = hex::decode(signature.trim()) else { return false };
    feed_mac(key, tenant, staff, version, expires).verify_slice(&sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_tokens_are_random_and_only_hashed() {
        let (a, ha) = new_token();
        let (b, hb) = new_token();
        assert_ne!(a, b);
        assert!(well_formed(&a) && well_formed(&b));
        assert_eq!((ha.len(), ha.clone()), (64, hash(&a)));
        assert_ne!(ha, hb);
        assert!(!well_formed("short"));
        assert!(!well_formed(&format!("{}!", &a[..42])));
        assert!(!well_formed(&format!("{a}x")));
    }

    #[test]
    fn feed_signatures_bind_every_part() {
        let (k1, k2) = (b"one key".as_slice(), b"another".as_slice());
        let (t, s) = (Uuid::now_v7(), Uuid::now_v7());
        let sig = feed_signature(k1, t, s, 1, 1_900_000_000);
        assert!(feed_signature_valid(k1, t, s, 1, 1_900_000_000, &sig));
        assert!(!feed_signature_valid(k2, t, s, 1, 1_900_000_000, &sig), "another key");
        assert!(!feed_signature_valid(k1, Uuid::now_v7(), s, 1, 1_900_000_000, &sig), "another tenant");
        assert!(!feed_signature_valid(k1, t, Uuid::now_v7(), 1, 1_900_000_000, &sig), "another staff member");
        assert!(!feed_signature_valid(k1, t, s, 2, 1_900_000_000, &sig), "a newer feed version");
        assert!(!feed_signature_valid(k1, t, s, 1, 1_900_000_001, &sig), "a changed expiry");
        assert!(!feed_signature_valid(k1, t, s, 1, 1_900_000_000, &sig[..60]), "a truncated signature");
        assert!(!feed_signature_valid(k1, t, s, 1, 1_900_000_000, "zz"));
    }
}
