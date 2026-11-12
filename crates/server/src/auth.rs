//! Admin authentication, CSRF protection, client IPs and rate limiting.

use std::net::SocketAddr;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, Method};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "qd_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// `bytes` random bytes from the OS, hex-encoded.
pub fn random_token(bytes: usize) -> AppResult<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(AppError::internal)?;
    Ok(hex::encode(buf))
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Compare two strings without leaking, through timing, how many leading
/// characters match.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Argon2id hash in PHC string format (salt and parameters included).
/// Deliberately slow (tens of ms): run it with `spawn_blocking`.
pub fn hash_password(password: &str) -> Result<String, String> {
    Argon2::default().hash_password(password.as_bytes()).map(|h| h.to_string()).map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// `Set-Cookie` value for the session. HttpOnly keeps it away from scripts,
/// SameSite=Strict stops it being sent on cross-site requests, and the path
/// limits it to the admin API.
pub fn session_cookie(token: &str, max_age_secs: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/api/admin; Max-Age={max_age_secs}{secure}")
}

/// An authenticated admin. Adding this as a handler argument is what
/// protects a route: extraction fails with 401 when there is no valid
/// session, and with 403 when a state-changing request lacks the CSRF token.
pub struct AdminSession {
    pub admin_id: Uuid,
    pub username: String,
    pub csrf_token: String,
    pub token_hash: String,
}

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie_value(&parts.headers, SESSION_COOKIE).ok_or_else(AppError::unauthorized)?;
        let token_hash = sha256_hex(token.as_bytes());
        let row = db::auth::session(&state.pool, &token_hash).await?.ok_or_else(AppError::unauthorized)?;
        // Double-submit CSRF check: a cross-site page can make the browser
        // send the cookie, but it cannot read the token to put in a header.
        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            let sent = parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
            if !constant_time_eq(sent, &row.csrf_token) {
                return Err(AppError::forbidden("missing or invalid CSRF token"));
            }
        }
        Ok(AdminSession { admin_id: row.admin_id, username: row.username, csrf_token: row.csrf_token, token_hash })
    }
}

/// Best-effort client IP, for rate limiting and the audit log.
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if state.cfg.trust_proxy {
            // Some hosts put the real client address in a dedicated header
            // that their edge always overwrites.
            if let Some(name) = &state.cfg.client_ip_header {
                let ip = parts
                    .headers
                    .get(name.as_str())
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| s.parse::<std::net::IpAddr>().is_ok());
                if let Some(ip) = ip {
                    return Ok(ClientIp(ip));
                }
            }
            // The reverse proxy appends the address it saw, so the *last*
            // entry is the one we can trust; earlier ones are client-supplied.
            let forwarded = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .map(|s| s.trim().to_string())
                .filter(|s| s.parse::<std::net::IpAddr>().is_ok());
            if let Some(ip) = forwarded {
                return Ok(ClientIp(ip));
            }
        }
        let ip = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_else(|| "unknown".into());
        Ok(ClientIp(ip))
    }
}

/// Count a request against a limit; `Err(429)` once it is exceeded.
pub async fn rate_limit(state: &AppState, scope: &str, subject: &str, window_secs: i32, limit: i32) -> AppResult<()> {
    let key = format!("{scope}:{subject}");
    if db::misc::rate_limit_hit(&state.pool, &key, window_secs, limit).await? {
        Ok(())
    } else {
        state.metrics.rate_limited.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(AppError::rate_limited())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_round_trip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not-a-hash"));
        // Salted: the same password hashes differently each time.
        assert_ne!(h, hash_password("correct horse battery").unwrap());
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "a=1; qd_session=abc123; b=2".parse().unwrap());
        assert_eq!(cookie_value(&h, SESSION_COOKIE), Some("abc123"));
        assert_eq!(cookie_value(&h, "missing"), None);
        assert!(session_cookie("t", 60, true).contains("; Secure"));
        assert!(!session_cookie("t", 60, false).contains("Secure"));
        assert!(session_cookie("t", 60, false).contains("HttpOnly; SameSite=Strict"));
    }

    #[test]
    fn tokens_and_comparison() {
        let a = random_token(32).unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, random_token(32).unwrap());
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }
}
