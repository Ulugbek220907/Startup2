//! Health, readiness and metrics.

use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

use crate::state::AppState;

/// Liveness: the process is up and serving requests.
pub async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: dependencies are reachable. A load balancer or the deploy
/// script should wait for this before sending traffic.
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = db::ping(&state.pool).await.is_ok();
    let store_ok = state.store.ping().await.is_ok();
    let body = format!("database: {}\nstorage: {}\n", if db_ok { "ok" } else { "FAIL" }, if store_ok { "ok" } else { "FAIL" });
    (if db_ok && store_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, body)
}

/// Prometheus text format. Contains only counts, no personal data. Do not
/// expose it publicly anyway: the production Caddyfile blocks `/metrics`.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let m = &state.metrics;
    let mut out = String::new();
    let mut counter = |name: &str, help: &str, labels: &str, v: u64| {
        if !out.contains(&format!("# TYPE {name} ")) {
            out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n"));
        }
        out.push_str(&format!("{name}{labels} {v}\n"));
    };
    for (class, v) in [("2xx", &m.http_2xx), ("3xx", &m.http_3xx), ("4xx", &m.http_4xx), ("5xx", &m.http_5xx)] {
        counter("qiyiq_http_responses_total", "HTTP responses by status class.", &format!("{{class=\"{class}\"}}"), v.load(Ordering::Relaxed));
    }
    counter("qiyiq_jobs_done_total", "Background jobs completed.", "", m.jobs_done.load(Ordering::Relaxed));
    counter("qiyiq_jobs_failed_total", "Background job attempts that failed.", "", m.jobs_failed.load(Ordering::Relaxed));
    counter("qiyiq_orders_created_total", "Orders created.", "", m.orders_created.load(Ordering::Relaxed));
    counter("qiyiq_uploads_total", "Images uploaded.", "", m.uploads.load(Ordering::Relaxed));
    counter("qiyiq_tryon_generated_total", "Photo try-on images generated.", "", m.tryon_generated.load(Ordering::Relaxed));
    counter("qiyiq_rate_limited_total", "Requests rejected by rate limiting.", "", m.rate_limited.load(Ordering::Relaxed));
    if let Ok(counts) = db::jobs::counts(&state.pool).await {
        out.push_str("# HELP qiyiq_jobs Jobs in the queue table by status.\n# TYPE qiyiq_jobs gauge\n");
        for (status, n) in counts {
            out.push_str(&format!("qiyiq_jobs{{status=\"{status}\"}} {n}\n"));
        }
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out)
}
