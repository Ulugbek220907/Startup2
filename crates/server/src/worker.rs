//! Background job workers.
//!
//! Each worker loops: claim a job, run its handler, mark it done or failed.
//! Handlers must be idempotent because a job can run more than once (a
//! worker may crash after doing the work but before recording success).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::FixedOffset;
use design::fonts::EmbeddedFonts;
use design::{Color, GarmentKind, Side};
use domain::api::PrintAreaDto;
use domain::i18n::{t, tf};
use domain::order::OrderStatus;
use domain::{phone, Lang, Tiyin};
use providers::tryon::{TryOnError, TryOnRequest};
use renderer::sheet::{SheetInfo, SheetSide};
use serde::Deserialize;
use uuid::Uuid;

use crate::render_svc;
use crate::state::AppState;

/// How long a claimed job is leased before another worker may retry it.
/// Longer than the slowest job (a 300-DPI render or a try-on generation).
const LEASE_SECS: i32 = 600;
const MOCKUP_WIDTH: u32 = 900;

/// A job failure. `retryable: false` skips the remaining attempts.
pub struct JobError {
    pub message: String,
    pub retryable: bool,
}

impl JobError {
    fn fatal(m: impl std::fmt::Display) -> Self {
        JobError { message: m.to_string(), retryable: false }
    }
}

// Anything that converts into an AppError (database, storage, render) is a
// retryable job failure.
impl<E: Into<crate::error::AppError>> From<E> for JobError {
    fn from(e: E) -> Self {
        JobError { message: e.into().message, retryable: true }
    }
}

type JobResult = Result<(), JobError>;

pub fn spawn(state: AppState) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles: Vec<_> = (0..state.cfg.workers).map(|n| tokio::spawn(worker_loop(state.clone(), n))).collect();
    handles.push(tokio::spawn(scheduler(state)));
    handles
}

async fn worker_loop(state: AppState, n: usize) {
    tracing::info!(worker = n, "worker started");
    loop {
        match db::jobs::claim(&state.pool, LEASE_SECS).await {
            Ok(Some(job)) => run_job(&state, job).await,
            Ok(None) => {
                // Sleep until a job is announced, or poll every few seconds
                // for delayed/retried jobs.
                let _ = tokio::time::timeout(Duration::from_secs(3), state.job_notify.notified()).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "claiming a job failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Run one claimed job to completion. Public so tests can drain the queue
/// deterministically instead of waiting for background workers.
pub async fn run_job(state: &AppState, job: db::jobs::Job) {
    let started = std::time::Instant::now();
    let result = handle(state, &job).await;
    let ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(()) => {
            state.metrics.jobs_done.fetch_add(1, Ordering::Relaxed);
            tracing::info!(job = job.id, kind = %job.kind, ms, "job done");
            if let Err(e) = db::jobs::complete(&state.pool, job.id).await {
                tracing::error!(job = job.id, error = %e, "could not mark job done");
            }
        }
        Err(err) => {
            state.metrics.jobs_failed.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(job = job.id, kind = %job.kind, attempt = job.attempts, error = %err.message, "job failed");
            match db::jobs::fail(&state.pool, &job, &err.message, err.retryable).await {
                Ok(true) => on_give_up(state, &job).await,
                Ok(false) => {}
                Err(e) => tracing::error!(job = job.id, error = %e, "could not record job failure"),
            }
        }
    }
}

/// Claim and run jobs until none are due. For tests and one-shot tools.
pub async fn drain(state: &AppState) -> usize {
    let mut n = 0;
    while let Ok(Some(job)) = db::jobs::claim(&state.pool, LEASE_SECS).await {
        run_job(state, job).await;
        n += 1;
    }
    n
}

/// Final failure: surface it where a human will see it.
async fn on_give_up(state: &AppState, job: &db::jobs::Job) {
    #[derive(Deserialize)]
    struct Ids {
        order_id: Option<Uuid>,
        tryon_id: Option<Uuid>,
    }
    let Ok(ids) = serde_json::from_value::<Ids>(job.payload.clone()) else { return };
    if job.kind == db::jobs::RENDER_ORDER_FILES {
        if let Some(id) = ids.order_id {
            let _ = db::orders::set_files_status(&state.pool, id, "failed").await;
        }
    }
    if job.kind == db::jobs::TRYON_GENERATE {
        if let Some(id) = ids.tryon_id {
            let _ = db::tryon::set_failed(&state.pool, id, "tryon_failed").await;
        }
    }
}

/// Keeps the periodic clean-up job queued.
async fn scheduler(state: AppState) {
    loop {
        match db::jobs::pending_exists(&state.pool, db::jobs::PURGE_EXPIRED).await {
            Ok(false) => {
                if let Err(e) = db::jobs::enqueue(&state.pool, db::jobs::PURGE_EXPIRED, &serde_json::json!({}), 0, 3).await {
                    tracing::error!(error = %e, "could not schedule clean-up");
                }
                state.job_notify.notify_one();
            }
            Ok(true) => {}
            Err(e) => tracing::error!(error = %e, "scheduler query failed"),
        }
        tokio::time::sleep(Duration::from_secs(600)).await;
    }
}

fn payload<T: for<'de> Deserialize<'de>>(job: &db::jobs::Job) -> Result<T, JobError> {
    serde_json::from_value(job.payload.clone()).map_err(|e| JobError::fatal(format!("bad payload: {e}")))
}

async fn handle(state: &AppState, job: &db::jobs::Job) -> JobResult {
    #[derive(Deserialize)]
    struct DesignJob {
        design_id: Uuid,
    }
    #[derive(Deserialize)]
    struct OrderJob {
        order_id: Uuid,
    }
    #[derive(Deserialize)]
    struct TryOnJob {
        tryon_id: Uuid,
    }
    match job.kind.as_str() {
        db::jobs::RENDER_MOCKUP => render_design_mockups(state, payload::<DesignJob>(job)?.design_id).await,
        db::jobs::RENDER_ORDER_FILES => render_order_files(state, payload::<OrderJob>(job)?.order_id).await,
        db::jobs::MODERATE_DESIGN => moderate_design(state, payload::<DesignJob>(job)?.design_id).await,
        db::jobs::TRYON_GENERATE => tryon_generate(state, payload::<TryOnJob>(job)?.tryon_id, job.attempts >= job.max_attempts).await,
        db::jobs::NOTIFY_ADMIN => notify_admin(state, &job.payload).await,
        db::jobs::NOTIFY_CUSTOMER => notify_customer(state, &job.payload).await,
        db::jobs::PURGE_EXPIRED => purge_expired(state).await,
        other => Err(JobError::fatal(format!("unknown job kind {other}"))),
    }
}

// ------------------------------------------------------------ rendering

async fn put_png(state: &AppState, key: &str, bytes: Vec<u8>) -> Result<i64, JobError> {
    let n = bytes.len() as i64;
    state.store.put(key, Bytes::from(bytes), "image/png").await?;
    Ok(n)
}

/// Mockups of both sides of a saved design, for share previews and reports.
async fn render_design_mockups(state: &AppState, design_id: Uuid) -> JobResult {
    let Some(row) = db::designs::design_by_id(&state.pool, design_id).await? else {
        return Ok(()); // deleted meanwhile: nothing to do
    };
    for side in Side::ALL {
        match crate::routes::public::design_garment_mockup(state, &row, side, MOCKUP_WIDTH).await {
            Ok(png) => {
                put_png(state, &format!("mockups/{design_id}/{}.png", side.as_str()), png).await?;
            }
            // The garment has no print area on this side.
            Err(e) if e.status == axum::http::StatusCode::NOT_FOUND => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Mockups, 300-DPI print files and the production sheet for an order.
async fn render_order_files(state: &AppState, order_id: Uuid) -> JobResult {
    let Some(order) = db::orders::order_by_id(&state.pool, order_id).await? else { return Ok(()) };
    // Everything comes from the order's frozen snapshot, not the live catalog.
    let doc = Arc::new(design::migrate(order.design_doc.clone()).map_err(JobError::fatal)?);
    let areas: Vec<PrintAreaDto> = serde_json::from_value(order.areas.clone()).map_err(JobError::fatal)?;
    let kind = GarmentKind::parse(&order.garment_kind).ok_or_else(|| JobError::fatal("unknown garment kind"))?;
    let color = Color::parse(&order.color_hex).ok_or_else(|| JobError::fatal("bad colour"))?;
    let assets = Arc::new(render_svc::load_assets(state, &doc).await?);
    let lang = state.cfg.admin_lang;

    let mut sheet_sides = Vec::new();
    for side in Side::ALL {
        let Some(area) = areas.iter().find(|a| a.side == side).copied() else { continue };
        let geom = render_svc::geom(&area);
        let s = side.as_str();

        let mockup = render_svc::mockup_png(state, doc.clone(), assets.clone(), side, kind, color, geom, MOCKUP_WIDTH).await?;
        let mockup_for_sheet = mockup.clone();
        let key = format!("orders/{order_id}/mockup_{s}.png");
        let n = put_png(state, &key, mockup).await?;
        db::orders::upsert_file(&state.pool, order_id, &format!("mockup_{s}"), &key, "image/png", n).await?;

        let printed = renderer::printed_box_cm(&doc, side, area.width_cm, area.height_cm, &EmbeddedFonts);
        if printed.is_some() {
            let (d, a) = (doc.clone(), assets.clone());
            let _permit = state.render_gate.acquire().await.map_err(JobError::fatal)?;
            let png = tokio::task::spawn_blocking(move || {
                let pm = renderer::render_print(&d, side, area.width_cm, area.height_cm, renderer::PRINT_DPI, &EmbeddedFonts, &*a)?;
                renderer::encode_png(&pm, Some(renderer::PRINT_DPI))
            })
            .await
            .map_err(JobError::fatal)??;
            let key = format!("orders/{order_id}/print_{s}.png");
            let n = put_png(state, &key, png).await?;
            db::orders::upsert_file(&state.pool, order_id, &format!("print_{s}"), &key, "image/png", n).await?;
        }

        let label = t(lang, if side == Side::Front { "common.front" } else { "common.back_side" }).to_string();
        sheet_sides.push((label, area, printed.map(|r| (r.w, r.h)), mockup_for_sheet));
    }

    // Region name for the sheet, in the admin's language.
    let delivery = match order.region_slug.as_deref() {
        Some(slug) => {
            let regions = db::catalog::regions(&state.pool).await?;
            regions.iter().find(|r| r.slug == slug).map(|r| r.name.get(lang).to_string()).unwrap_or_else(|| slug.to_string())
        }
        None => t(lang, "order.pickup").to_string(),
    };
    // Uzbekistan is UTC+5 all year (no daylight saving).
    let tz = FixedOffset::east_opt(5 * 3600).ok_or_else(|| JobError::fatal("timezone"))?;
    let info_base = (
        order.code.clone(),
        order.created_at.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
        order.garment_name.clone(),
        order.color_name.clone(),
        order.size.clone(),
        order.quantity as u32,
        order.notes.clone(),
    );
    let brand = state.cfg.brand.clone();
    let admin_url = format!("{}/admin/#/orders/{order_id}", state.cfg.public_base_url);
    let finish = t(lang, "editor.finish_standard").to_string();
    let pdf = tokio::task::spawn_blocking(move || {
        let sides = sheet_sides
            .into_iter()
            .map(|(label, area, printed_cm, png)| SheetSide {
                label,
                area_w_cm: area.width_cm,
                area_h_cm: area.height_cm,
                printed_cm,
                mockup: renderer::Pixmap::decode_png(&png).ok(),
            })
            .collect();
        let (order_code, created_at, garment, color_name, size, quantity, notes) = info_base;
        renderer::sheet::production_sheet(
            &SheetInfo { brand, order_code, created_at, garment, color_name, color, size, quantity, finish, delivery, notes, admin_url, sides, lang },
            &EmbeddedFonts,
        )
    })
    .await
    .map_err(JobError::fatal)??;
    let key = format!("orders/{order_id}/sheet.pdf");
    let n = pdf.len() as i64;
    state.store.put(&key, Bytes::from(pdf), "application/pdf").await?;
    db::orders::upsert_file(&state.pool, order_id, "sheet", &key, "application/pdf", n).await?;
    db::orders::set_files_status(&state.pool, order_id, "ready").await?;
    Ok(())
}

// ----------------------------------------------------------- moderation

async fn moderate_design(state: &AppState, design_id: Uuid) -> JobResult {
    let Some(row) = db::designs::design_by_id(&state.pool, design_id).await? else { return Ok(()) };
    let doc = design::migrate(row.doc).map_err(JobError::fatal)?;

    let blocklist = db::catalog::blocklist(&state.pool).await?;
    if let Some(term) = domain::moderation::find_blocked(&doc.all_text(), blocklist.iter().map(String::as_str)) {
        db::designs::set_auto_moderation(&state.pool, design_id, true, Some(&format!("blocked word: {term}"))).await?;
        return Ok(());
    }
    let ids: Vec<Uuid> = doc.asset_ids().iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
    for asset in db::designs::assets_by_ids(&state.pool, &ids).await? {
        let Some(bytes) = state.store.get(&asset.preview_key).await? else { continue };
        let verdict = state.moderator.check(&bytes).await.map_err(|e| JobError { message: e.to_string(), retryable: true })?;
        if verdict.flagged {
            let reason = verdict.reason.unwrap_or_else(|| "image flagged".into());
            db::designs::set_auto_moderation(&state.pool, design_id, true, Some(&reason)).await?;
            return Ok(());
        }
    }
    db::designs::set_auto_moderation(&state.pool, design_id, false, None).await?;
    Ok(())
}

// --------------------------------------------------------------- try-on

async fn tryon_generate(state: &AppState, id: Uuid, last_attempt: bool) -> JobResult {
    let Some(row) = db::tryon::get(&state.pool, id).await? else { return Ok(()) };
    if !db::tryon::set_running(&state.pool, id).await? {
        return Ok(()); // deleted by the customer, or already finished
    }
    let (Some(photo_key), Some(garment_key)) = (row.photo_key, row.garment_key) else { return Ok(()) };
    let (Some(person_jpeg), Some(garment_png)) = (state.store.get(&photo_key).await?, state.store.get(&garment_key).await?) else {
        db::tryon::set_failed(&state.pool, id, "tryon_failed").await?;
        return Ok(());
    };

    match state.tryon.generate(TryOnRequest { person_jpeg, garment_png }).await {
        Ok(out) => {
            let ext = if out.mime == "image/png" { "png" } else { "jpg" };
            let key = format!("tryon/{id}/result.{ext}");
            state.store.put(&key, out.image, out.mime).await?;
            if db::tryon::set_done(&state.pool, id, &key, out.mime, state.tryon.cost_per_image().0).await? {
                state.metrics.tryon_generated.fetch_add(1, Ordering::Relaxed);
            } else {
                // Deleted while generating: do not keep the result.
                state.store.delete(&key).await?;
            }
            Ok(())
        }
        Err(TryOnError::Rejected(detail)) => {
            // The provider's detail may describe the photo; keep it out of
            // the customer-visible error and log only its length.
            tracing::info!(tryon = %id, detail_len = detail.len(), "try-on input rejected by provider");
            db::tryon::set_failed(&state.pool, id, "tryon_failed").await?;
            Ok(())
        }
        Err(e) => {
            if last_attempt {
                db::tryon::set_failed(&state.pool, id, "tryon_failed").await?;
            }
            Err(JobError { message: e.to_string(), retryable: true })
        }
    }
}

// -------------------------------------------------------- notifications

async fn notify_admin(state: &AppState, payload: &serde_json::Value) -> JobResult {
    let lang = state.cfg.admin_lang;
    // With the mock messenger there is no real chat; use id 0 so the
    // message is still recorded and visible in logs/tests.
    let chat_id = match &state.cfg.telegram {
        Some(tg) => match tg.admin_chat_id {
            Some(id) => id,
            None => return Ok(()), // bot configured without an admin chat
        },
        None => 0,
    };
    let text = if let Some(order_id) = payload.get("order_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()) {
        let Some(o) = db::orders::order_by_id(&state.pool, order_id).await? else { return Ok(()) };
        tf(
            lang,
            "admin.new_order_alert",
            &[
                ("code", &o.code),
                ("garment", &o.garment_name),
                ("size", &o.size),
                ("qty", &o.quantity.to_string()),
                ("total", &Tiyin(o.total_tiyin).format_sum()),
                ("phone", &phone::format_display(o.phone.as_deref().unwrap_or("-"))),
                ("url", &format!("{}/admin/#/orders/{}", state.cfg.public_base_url, o.id)),
            ],
        )
    } else if let Some(slug) = payload.get("report_design_slug").and_then(|v| v.as_str()) {
        format!("{}: {}\n{}/admin/#/reports", t(lang, "admin.reports"), slug, state.cfg.public_base_url)
    } else {
        return Err(JobError::fatal("unknown admin notification"));
    };
    state.messenger.send(chat_id, &text).await.map_err(|e| JobError { message: e.to_string(), retryable: true })
}

async fn notify_customer(state: &AppState, payload: &serde_json::Value) -> JobResult {
    #[derive(Deserialize)]
    struct P {
        order_id: Uuid,
        event: String,
    }
    let p: P = serde_json::from_value(payload.clone()).map_err(JobError::fatal)?;
    let Some(o) = db::orders::order_by_id(&state.pool, p.order_id).await? else { return Ok(()) };
    let Some(phone) = o.phone.as_deref() else { return Ok(()) };
    // Customers are notified on Telegram if they linked their phone to the
    // bot. Without a link there is no channel in v1 (SMS arrives in v2).
    let Some(chat_id) = db::orders::customer_by_phone(&state.pool, phone).await?.and_then(|c| c.telegram_chat_id) else {
        return Ok(());
    };
    let lang = Lang::parse(&o.lang).unwrap_or_default();
    let text = match p.event.as_str() {
        "received" => tf(lang, "bot.order_received", &[("code", &o.code), ("total", &Tiyin(o.total_tiyin).format_sum())]),
        _ => {
            let status = OrderStatus::parse(&o.status).map(|s| t(lang, s.i18n_key())).unwrap_or("");
            tf(lang, "bot.status_changed", &[("code", &o.code), ("status", status)])
        }
    };
    state.messenger.send(chat_id, &text).await.map_err(|e| JobError { message: e.to_string(), retryable: true })
}

// ------------------------------------------------------------- clean-up

/// Data retention: delete what is past its lifetime. See docs/PRIVACY.md.
async fn purge_expired(state: &AppState) -> JobResult {
    let mut tryon = 0;
    loop {
        let ids = db::tryon::expired(&state.pool, 100).await?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            crate::routes::tryon::delete_files(state, id).await?;
            tryon += 1;
        }
    }
    let sessions = db::auth::purge_sessions(&state.pool).await?;
    let limits = db::misc::purge_rate_limits(&state.pool).await?;
    let jobs = db::jobs::purge_finished(&state.pool, 14).await?;
    let drafts = db::designs::purge_stale_drafts(&state.pool, state.cfg.draft_ttl_days).await?;
    let mut assets = 0;
    for a in db::designs::orphan_assets(&state.pool, state.cfg.draft_ttl_days).await? {
        state.store.delete(&a.storage_key).await?;
        state.store.delete(&a.preview_key).await?;
        db::designs::delete_asset(&state.pool, a.id).await?;
        assets += 1;
    }
    tracing::info!(tryon, sessions, limits, jobs, drafts, assets, "clean-up finished");
    Ok(())
}
