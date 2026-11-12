//! Shared application state handed to every handler and worker.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use providers::bg::BackgroundRemover;
use providers::delivery::DeliveryQuoter;
use providers::messenger::Messenger;
use providers::moderation::ImageModerator;
use providers::sms::SmsSender;
use providers::tryon::TryOnProvider;
use storage::ObjectStore;
use tokio::sync::{Notify, Semaphore};

use crate::config::Config;

/// Cheap to clone: everything lives behind one `Arc`.
#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub pool: db::PgPool,
    pub store: Arc<dyn ObjectStore>,
    pub tryon: Arc<dyn TryOnProvider>,
    pub moderator: Arc<dyn ImageModerator>,
    pub bg: Arc<dyn BackgroundRemover>,
    pub messenger: Arc<dyn Messenger>,
    pub sms: Arc<dyn SmsSender>,
    pub delivery: Arc<dyn DeliveryQuoter>,
    /// Limits concurrent renders: a 300-DPI print bitmap is ~70 MB.
    pub render_gate: Semaphore,
    /// Wakes idle workers as soon as a job is queued.
    pub job_notify: Notify,
    pub metrics: Metrics,
    /// `script-src` hashes of inline scripts in the built `index.html`.
    pub csp_script_hashes: String,
}

// `Deref` lets handlers write `state.pool` instead of `state.0.pool`.
impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Process-wide counters exported at `/metrics`.
#[derive(Default)]
pub struct Metrics {
    pub http_2xx: AtomicU64,
    pub http_3xx: AtomicU64,
    pub http_4xx: AtomicU64,
    pub http_5xx: AtomicU64,
    pub jobs_done: AtomicU64,
    pub jobs_failed: AtomicU64,
    pub orders_created: AtomicU64,
    pub uploads: AtomicU64,
    pub tryon_generated: AtomicU64,
    pub rate_limited: AtomicU64,
}
