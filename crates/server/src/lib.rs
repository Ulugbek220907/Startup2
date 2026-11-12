//! The backend as a library, so integration tests can build the exact same
//! router and state as the real binary.

pub mod auth;
pub mod bot;
pub mod config;
pub mod error;
pub mod render_svc;
pub mod routes;
pub mod state;
pub mod telegram;
pub mod uploads;
pub mod worker;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine as _;
use providers::bg::KeyColorRemover;
use providers::delivery::TableDelivery;
use providers::messenger::{LogMessenger, Messenger};
use providers::moderation::{MockMode, MockModerator};
use providers::sms::LogSms;
use providers::tryon::{FashnConfig, FashnTryOn, MockTryOn, TryOnProvider};
use sha2::{Digest, Sha256};
use storage::{FsStore, MemStore, ObjectStore, S3Store};
use tokio::sync::{Notify, Semaphore};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::config::{Config, StorageConfig, TryOnProviderKind};
use crate::state::{AppState, Inner, Metrics};

/// JSON bodies (design documents with freehand drawings can be large-ish).
const JSON_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Build providers from configuration: mocks unless a real one is configured.
pub fn build_state(cfg: Config, pool: db::PgPool) -> anyhow::Result<AppState> {
    let store: Arc<dyn ObjectStore> = match &cfg.storage {
        StorageConfig::S3(s3) => Arc::new(S3Store::new(s3)?),
        StorageConfig::Fs(dir) => Arc::new(FsStore::new(dir)),
        StorageConfig::Memory => Arc::new(MemStore::new()),
    };
    let tryon: Arc<dyn TryOnProvider> = match &cfg.tryon.provider {
        TryOnProviderKind::Mock => Arc::new(MockTryOn::default()),
        TryOnProviderKind::Fashn { api_key, base_url, mode } => Arc::new(FashnTryOn::new(FashnConfig {
            api_key: api_key.clone(),
            base_url: base_url.clone(),
            mode: mode.clone(),
            timeout: cfg.tryon.timeout,
            poll_interval: Duration::from_secs(2),
            cost_per_image: cfg.tryon.cost_per_image,
        })?),
    };
    let messenger: Arc<dyn Messenger> = match &cfg.telegram {
        Some(tg) => Arc::new(bot::TelegramMessenger::new(&tg.bot_token)),
        None => Arc::new(LogMessenger::new()),
    };
    Ok(assemble_state(cfg, pool, store, tryon, messenger))
}

/// Assemble state from explicit parts (tests pass in-memory fakes).
pub fn assemble_state(cfg: Config, pool: db::PgPool, store: Arc<dyn ObjectStore>, tryon: Arc<dyn TryOnProvider>, messenger: Arc<dyn Messenger>) -> AppState {
    let mode = if cfg.moderation_mock_flag_all { MockMode::FlagAll } else { MockMode::AllowAll };
    let moderator = Arc::new(MockModerator::new(mode, cfg.moderation_mock_flag_sha256.clone()));
    let csp_script_hashes = inline_script_hashes(&cfg.static_dir);
    AppState(Arc::new(Inner {
        pool,
        store,
        tryon,
        moderator,
        bg: Arc::new(KeyColorRemover::default()),
        messenger,
        sms: Arc::new(LogSms::new()),
        delivery: Arc::new(TableDelivery),
        // One print render at a time keeps peak memory near 150 MB.
        render_gate: Semaphore::new(1),
        job_notify: Notify::new(),
        metrics: Metrics::default(),
        csp_script_hashes,
        cfg,
    }))
}

/// Create or update the admin account from `ADMIN_USERNAME`/`ADMIN_PASSWORD`.
pub async fn bootstrap_admin(state: &AppState) -> anyhow::Result<()> {
    if let Some((username, password)) = &state.cfg.admin_bootstrap {
        let password = password.clone();
        let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password)).await?.map_err(anyhow::Error::msg)?;
        db::auth::upsert_admin(&state.pool, username, &hash).await?;
        tracing::info!(username = %username, "admin account ready");
    } else if db::auth::count_admins(&state.pool).await? == 0 {
        tracing::warn!("no admin account exists; set ADMIN_USERNAME and ADMIN_PASSWORD to create one");
    }
    Ok(())
}

/// Create one example order on an empty database (`SEED_DEMO=true`), so the
/// admin page has something to show right after `docker compose up`. It
/// goes through the same code path as a real order.
pub async fn seed_demo(state: &AppState) -> anyhow::Result<()> {
    use axum::extract::State;
    use domain::api::{CreateOrderRequest, DeliveryChoice};
    let existing: i64 = db::sqlx::query_scalar("SELECT count(*) FROM orders").fetch_one(&state.pool).await?;
    if existing > 0 {
        return Ok(());
    }
    let mut doc = design::DesignDoc::new("hoodie", "black");
    doc.front = design::templates::find("city_arch").map(|t| t.layers(30.0, 35.0, design::Color::WHITE)).unwrap_or_default();
    let req = CreateOrderRequest {
        doc,
        design_slug: None,
        size: "L".into(),
        quantity: 2,
        phone: "+998901234567".into(),
        customer_name: Some("Demo mijoz".into()),
        delivery: DeliveryChoice { method: domain::order::DeliveryMethod::Delivery, region: Some("tashkent-city".into()) },
        address: Some("Namuna koʻchasi 1 (demo buyurtma)".into()),
        notes: Some("Bu namuna buyurtma. Oʻchirib tashlashingiz mumkin.".into()),
        lang: domain::Lang::Uz,
        accept_rights: true,
    };
    match routes::public::create_order(State(state.clone()), auth::ClientIp("seed".into()), axum::Json(req)).await {
        Ok((_, axum::Json(order))) => tracing::info!(code = %order.code, "demo order created"),
        Err(e) => anyhow::bail!("{}", e.message),
    }
    Ok(())
}

/// CSP `'sha256-…'` sources for the inline scripts in the built HTML. The
/// bundler emits a small inline module that boots the WASM; hashing it at
/// startup lets the policy forbid every other inline script.
fn inline_script_hashes(static_dir: &str) -> String {
    let mut hashes = Vec::new();
    for file in ["index.html", "admin/index.html"] {
        let Ok(html) = std::fs::read_to_string(std::path::Path::new(static_dir).join(file)) else { continue };
        hashes.extend(script_hashes_in(&html));
    }
    hashes.sort();
    hashes.dedup();
    hashes.join(" ")
}

fn script_hashes_in(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let after = &rest[start..];
        let Some(tag_end) = after.find('>') else { break };
        let Some(close) = after.find("</script>") else { break };
        let tag = &after[..tag_end];
        if close > tag_end && !tag.contains(" src=") {
            let body = &after[tag_end + 1..close];
            if !body.trim().is_empty() {
                let digest = Sha256::digest(body.as_bytes());
                out.push(format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(digest)));
            }
        }
        rest = &after[close + 9..];
    }
    out
}

fn csp(state: &AppState, framed_by_telegram: bool) -> String {
    // Telegram's web clients show Mini Apps in an iframe, so the customer
    // app must allow being framed by Telegram; admin and API never are.
    let ancestors = if framed_by_telegram { "'self' https://web.telegram.org https://*.telegram.org" } else { "'none'" };
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval' https://telegram.org {}; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; connect-src 'self' data: blob:; worker-src 'self' blob:; \
         object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors {ancestors}",
        state.csp_script_hashes
    )
}

/// Does the last path segment look like a fingerprinted build artefact
/// (`name-0123456789abcdef.js`)? Those never change and can be cached forever.
fn is_fingerprinted(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or("");
    let Some((stem, _ext)) = file.rsplit_once('.') else { return false };
    let stem = stem.strip_suffix("_bg").unwrap_or(stem);
    stem.rsplit_once('-').is_some_and(|(_, h)| h.len() >= 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Adds security and caching headers to every response and counts it.
async fn finish_response(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let mut res = next.run(req).await;

    let counter = match res.status().as_u16() {
        200..=299 => &state.metrics.http_2xx,
        300..=399 => &state.metrics.http_3xx,
        400..=499 => &state.metrics.http_4xx,
        _ => &state.metrics.http_5xx,
    };
    counter.fetch_add(1, Ordering::Relaxed);

    let is_api = path.starts_with("/api/");
    let restricted = is_api || path.starts_with("/admin");
    let h = res.headers_mut();
    let mut set = |name: HeaderName, value: &str| {
        if let Ok(v) = HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(header::REFERRER_POLICY, "strict-origin-when-cross-origin");
    set(HeaderName::from_static("permissions-policy"), "camera=(self), microphone=(), geolocation=(), payment=()");
    set(header::CONTENT_SECURITY_POLICY, &csp(&state, !restricted));
    if restricted {
        set(header::X_FRAME_OPTIONS, "DENY");
    }
    if state.cfg.cookie_secure {
        set(header::STRICT_TRANSPORT_SECURITY, "max-age=31536000; includeSubDomains");
    }
    if !h.contains_key(header::CACHE_CONTROL) {
        let cache = if is_api {
            "no-store"
        } else if is_fingerprinted(&path) {
            "public, max-age=31536000, immutable"
        } else if path.starts_with("/fonts/") || path.starts_with("/vendor/") || path.starts_with("/icons/") {
            "public, max-age=604800"
        } else {
            // HTML, the service worker and the manifest must always be fresh.
            "no-cache"
        };
        if let Ok(v) = HeaderValue::from_str(cache) {
            h.insert(header::CACHE_CONTROL, v);
        }
    }
    res
}

fn cors(cfg: &Config) -> CorsLayer {
    // Only the listed origins (the mobile app shells) may call the API from
    // another origin. Cookies are never allowed cross-origin, so the admin
    // session cannot be used from a foreign page.
    let origins: Vec<HeaderValue> = cfg.cors_allowed_origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("x-device-id"),
            HeaderName::from_static("x-consent"),
            HeaderName::from_static("x-edit-token"),
        ])
        .max_age(Duration::from_secs(3600))
}

/// The complete application: API, static web app and middleware.
pub fn build_app(state: AppState) -> Router {
    use routes::{admin, ops, public, tryon};
    let upload_limit = DefaultBodyLimit::max(state.cfg.max_upload_bytes + 1024);

    let api = Router::new()
        .route("/catalog", get(public::catalog))
        .route("/assets", post(public::upload_asset).layer(upload_limit))
        .route("/assets/{id}/preview", get(public::asset_preview))
        .route("/assets/{id}/remove-bg", post(public::remove_background))
        .route("/designs", post(public::create_design))
        .route("/designs/{slug}", get(public::get_design).put(public::update_design))
        .route("/designs/{slug}/render", post(public::render_design))
        .route("/designs/{slug}/mockup/{side}", get(public::design_mockup))
        .route("/designs/{slug}/report", post(public::report_design))
        .route("/jobs/{id}", get(public::job_status))
        .route("/price/quote", post(public::quote))
        .route("/orders", post(public::create_order))
        .route("/orders/lookup", post(public::lookup_order))
        .route("/me/export", post(public::export_data))
        .route("/me/delete", post(public::delete_data))
        .route("/telegram/me", post(public::telegram_me))
        .route("/tryon", post(tryon::create).layer(upload_limit))
        .route("/tryon/{id}", get(tryon::get).delete(tryon::delete))
        .route("/tryon/{id}/result", get(tryon::result))
        .route("/admin/login", post(admin::login))
        .route("/admin/logout", post(admin::logout))
        .route("/admin/me", get(admin::me))
        .route("/admin/orders", get(admin::list_orders))
        .route("/admin/orders/{id}", get(admin::get_order))
        .route("/admin/orders/{id}/status", post(admin::change_status))
        .route("/admin/orders/{id}/rerender", post(admin::rerender))
        .route("/admin/orders/{id}/files/{kind}", get(admin::download_file))
        .route("/admin/moderation", get(admin::moderation_queue))
        .route("/admin/designs/{id}/preview", get(admin::design_preview))
        .route("/admin/designs/{id}/moderation", post(admin::decide_moderation))
        .route("/admin/reports", get(admin::reports))
        .route("/admin/reports/{id}", post(admin::decide_report))
        .route("/admin/pricing", get(admin::get_pricing).put(admin::put_pricing))
        .route("/admin/blocklist", get(admin::get_blocklist).put(admin::put_blocklist))
        .route("/admin/audit", get(admin::audit_log))
        .fallback(|| async { error::AppError::not_found() })
        .layer(cors(&state.cfg))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT));

    // Static files. Unknown paths fall back to index.html so client-side
    // routes like /design/hoodie work on reload.
    let dir = std::path::PathBuf::from(&state.cfg.static_dir);
    let admin_files = ServeDir::new(dir.join("admin")).fallback(ServeFile::new(dir.join("admin/index.html")));
    let web_files = ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")));

    Router::new()
        .route("/healthz", get(ops::healthz))
        .route("/readyz", get(ops::readyz))
        .route("/metrics", get(ops::metrics))
        .nest("/api", api)
        .nest_service("/admin", admin_files)
        .fallback_service(web_files)
        .layer(axum::middleware::from_fn_with_state(state.clone(), finish_response))
        .layer(CompressionLayer::new())
        .layer(
            // The span carries the request id and the path only: query
            // strings can contain phone numbers (admin search).
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                let id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
                tracing::info_span!("http", method = %req.method(), path = %req.uri().path(), request_id = %id)
            }),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(90)))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_only_inline_scripts() {
        let html = r#"<html><script src="/a.js"></script><script type="module">init();</script><script></script></html>"#;
        let hashes = script_hashes_in(html);
        assert_eq!(hashes.len(), 1);
        let want = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(b"init();"));
        assert_eq!(hashes[0], format!("'sha256-{want}'"));
        assert!(script_hashes_in("<p>no scripts</p>").is_empty());
        assert!(script_hashes_in("<script>unterminated").is_empty());
    }

    #[test]
    fn fingerprint_detection() {
        assert!(is_fingerprinted("/web-1a2b3c4d5e6f7a8b.js"));
        assert!(is_fingerprinted("/web-1a2b3c4d5e6f7a8b_bg.wasm"));
        assert!(is_fingerprinted("/styles-00112233445566778899.css"));
        assert!(!is_fingerprinted("/index.html"));
        assert!(!is_fingerprinted("/sw.js"));
        assert!(!is_fingerprinted("/design/hoodie"));
        assert!(!is_fingerprinted("/fonts/Montserrat-Bold.ttf"));
    }
}
