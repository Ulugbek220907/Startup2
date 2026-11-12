//! Telegram Mini App `initData` verification.
//!
//! When the web app runs inside Telegram, Telegram hands it a signed query
//! string describing the user. The signature is an HMAC keyed with the bot
//! token, so only our server can check it and nobody can forge another
//! user's id. Algorithm (from Telegram's "Validating data received via the
//! Mini App" documentation):
//!
//! 1. take all `key=value` pairs except `hash`, sort by key, join with `\n`
//! 2. `secret = HMAC_SHA256(key = "WebAppData", message = bot_token)`
//! 3. valid if `hex(HMAC_SHA256(key = secret, message = joined)) == hash`

use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TgUser {
    pub id: i64,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub language_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitDataError {
    Malformed,
    BadSignature,
    Expired,
}

/// How old `auth_date` may be. Telegram re-issues initData whenever the
/// Mini App is opened, so a day is generous.
pub const MAX_AGE_SECS: i64 = 24 * 3600;

fn hmac(key: &[u8], msg: &[u8]) -> Result<HmacSha256, InitDataError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| InitDataError::Malformed)?;
    mac.update(msg);
    Ok(mac)
}

fn decode(s: &str) -> Result<String, InitDataError> {
    // Query strings encode spaces as '+'.
    let plus = s.replace('+', " ");
    percent_decode_str(&plus).decode_utf8().map(|c| c.into_owned()).map_err(|_| InitDataError::Malformed)
}

pub fn verify_init_data(init_data: &str, bot_token: &str, now_unix: i64) -> Result<TgUser, InitDataError> {
    if init_data.len() > 8192 {
        return Err(InitDataError::Malformed);
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut hash = None;
    for part in init_data.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').ok_or(InitDataError::Malformed)?;
        let (k, v) = (decode(k)?, decode(v)?);
        if k == "hash" {
            hash = Some(v);
        } else {
            pairs.push((k, v));
        }
    }
    let hash = hex::decode(hash.ok_or(InitDataError::Malformed)?).map_err(|_| InitDataError::Malformed)?;
    pairs.sort();
    let check = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");

    let secret = hmac(b"WebAppData", bot_token.as_bytes())?.finalize().into_bytes();
    // `verify_slice` compares in constant time.
    hmac(&secret, check.as_bytes())?.verify_slice(&hash).map_err(|_| InitDataError::BadSignature)?;

    let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let auth_date: i64 = get("auth_date").and_then(|v| v.parse().ok()).ok_or(InitDataError::Malformed)?;
    if now_unix - auth_date > MAX_AGE_SECS || auth_date - now_unix > 300 {
        return Err(InitDataError::Expired);
    }
    serde_json::from_str(get("user").ok_or(InitDataError::Malformed)?).map_err(|_| InitDataError::Malformed)
}

/// Build a correctly signed initData string. Used by tests and by the
/// local development helper; never needed in production.
pub fn sign_init_data(fields: &[(&str, &str)], bot_token: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    let mut sorted: Vec<(&str, &str)> = fields.to_vec();
    sorted.sort();
    let check = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
    let secret = hmac(b"WebAppData", bot_token.as_bytes()).map(|m| m.finalize().into_bytes().to_vec()).unwrap_or_default();
    let sig = hmac(&secret, check.as_bytes()).map(|m| hex::encode(m.finalize().into_bytes())).unwrap_or_default();
    let mut out: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={}", utf8_percent_encode(v, NON_ALPHANUMERIC))).collect();
    out.push(format!("hash={sig}"));
    out.join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "123456:TEST-TOKEN-not-a-real-bot";
    const USER: &str = r#"{"id":4242,"first_name":"Ulugʻbek","language_code":"uz"}"#;

    #[test]
    fn accepts_valid_data() {
        let data = sign_init_data(&[("auth_date", "1000000"), ("query_id", "AAE"), ("user", USER)], TOKEN);
        let u = verify_init_data(&data, TOKEN, 1_000_100).unwrap();
        assert_eq!(u.id, 4242);
        assert_eq!(u.first_name.as_deref(), Some("Ulugʻbek"));
        assert_eq!(u.language_code.as_deref(), Some("uz"));
    }

    #[test]
    fn rejects_tampering_wrong_token_and_old_data() {
        let data = sign_init_data(&[("auth_date", "1000000"), ("user", USER)], TOKEN);
        // Change the user id but keep the old signature.
        let forged = data.replace("4242", "1");
        assert_eq!(verify_init_data(&forged, TOKEN, 1_000_100), Err(InitDataError::BadSignature));
        assert_eq!(verify_init_data(&data, "999:other-bot", 1_000_100), Err(InitDataError::BadSignature));
        assert_eq!(verify_init_data(&data, TOKEN, 1_000_000 + MAX_AGE_SECS + 1), Err(InitDataError::Expired));
        assert_eq!(verify_init_data("user=x", TOKEN, 0), Err(InitDataError::Malformed));
        assert_eq!(verify_init_data("", TOKEN, 0), Err(InitDataError::Malformed));
        assert_eq!(verify_init_data(&format!("{data}&extra=1"), TOKEN, 1_000_100), Err(InitDataError::BadSignature));
    }

    #[test]
    fn matches_an_independently_computed_signature() {
        // Computed by hand from the documented algorithm, not via
        // `sign_init_data`, to make sure both sides are not wrong together.
        let check = "auth_date=1700000000\nuser={\"id\":1}";
        let secret = hmac(b"WebAppData", b"1:abc").unwrap().finalize().into_bytes();
        let sig = hex::encode(hmac(&secret, check.as_bytes()).unwrap().finalize().into_bytes());
        let data = format!("user=%7B%22id%22%3A1%7D&auth_date=1700000000&hash={sig}");
        assert_eq!(verify_init_data(&data, "1:abc", 1_700_000_010).unwrap().id, 1);
    }
}
