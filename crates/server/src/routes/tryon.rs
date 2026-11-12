//! Photo try-on (Tier B) endpoints.
//!
//! Privacy rules enforced here: explicit consent is required before a
//! photo is accepted; the photo is re-encoded (EXIF stripped) and stored
//! only until its TTL; the customer can delete it at any time; results are
//! only readable with the unguessable job id.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use design::Side;
use domain::api::{DeletedDto, TryOnDto, TryOnState};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{rate_limit, sha256_hex, ClientIp};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateParams {
    /// Share slug of the saved design.
    pub design: String,
    #[serde(default)]
    pub side: Option<String>,
}

/// The browser generates a random device id once and sends it with every
/// try-on request; it scopes rate limits and result caching to one device
/// without requiring an account.
fn device_id(headers: &HeaderMap) -> AppResult<String> {
    let id = headers.get("x-device-id").and_then(|v| v.to_str().ok()).unwrap_or("");
    let ok = (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(id.to_string())
    } else {
        Err(AppError::invalid("missing or invalid X-Device-Id header"))
    }
}

fn dto(row: &db::tryon::TryOnRow) -> TryOnDto {
    let status = TryOnState::parse(&row.status).unwrap_or(TryOnState::Failed);
    TryOnDto {
        id: row.id.to_string(),
        status,
        result_url: (status == TryOnState::Done && row.result_key.is_some()).then(|| format!("/api/tryon/{}/result", row.id)),
        error: row.error.clone(),
        expires_at: db::ts(row.expires_at),
    }
}

/// `POST /api/tryon?design=<slug>&side=front` with the photo as the body.
pub async fn create(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Query(params): Query<CreateParams>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<(StatusCode, Json<TryOnDto>)> {
    // Consent first: without it the photo is not even decoded.
    if headers.get("x-consent").and_then(|v| v.to_str().ok()) != Some("yes") {
        return Err(AppError::bad_request("tryon_consent", "consent is required before uploading a photo"));
    }
    let device = device_id(&headers)?;
    rate_limit(&state, "tryon-ip", &ip, 3600, 15).await?;
    if body.len() > state.cfg.max_upload_bytes {
        return Err(AppError::bad_request("upload_too_large", "file is too large"));
    }
    let side = match params.side.as_deref() {
        None => Side::Front,
        Some(s) => Side::parse(s).ok_or_else(|| AppError::invalid("side must be front or back"))?,
    };
    let design = db::designs::design_by_slug(&state.pool, &params.design)
        .await?
        .filter(|d| d.moderation_status != "rejected")
        .ok_or_else(AppError::not_found)?;

    let img = crate::uploads::decode(body).await.map_err(|e| AppError::bad_request(e.code(), "photo was rejected"))?;
    providers::tryon::precheck_photo(&img).map_err(|issue| AppError::bad_request(issue.code(), "photo failed the quality check"))?;
    // Downscale and re-encode: smaller upload to the provider, no metadata.
    let photo = tokio::task::spawn_blocking(move || {
        let img = if img.width().max(img.height()) > 1536 { img.thumbnail(1536, 1536) } else { img };
        crate::uploads::encode_jpeg(&img, 90)
    })
    .await?
    .map_err(|e| AppError::bad_request(e.code(), "photo could not be processed"))?;

    // Identical photo + design from the same device: reuse the result.
    let cache_key = sha256_hex(format!("{}:{}:{}", sha256_hex(&photo), sha256_hex(design.doc.to_string().as_bytes()), side.as_str()).as_bytes());
    if let Some(hit) = db::tryon::find_cached(&state.pool, &cache_key, &device).await? {
        return Ok((StatusCode::OK, Json(dto(&hit))));
    }

    // Limits are checked only when a new generation would actually be paid for.
    let t = &state.cfg.tryon;
    let cost = state.tryon.cost_per_image();
    let (count_today, cost_today) = db::tryon::usage_today(&state.pool, cost.0).await?;
    let over_device = db::tryon::count_recent_for_device(&state.pool, &device).await? >= t.per_device_daily;
    let over_global = count_today >= t.global_daily;
    let over_cost = cost.0 > 0 && cost_today + cost.0 > t.daily_cost_cap.0;
    if over_device || over_global || over_cost {
        tracing::warn!(over_device, over_global, over_cost, "try-on limit reached");
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, "tryon_limit", "try-on limit reached"));
    }

    let id = Uuid::new_v4();
    let photo_key = format!("tryon/{id}/photo.jpg");
    let garment_key = format!("tryon/{id}/garment.png");
    let garment = crate::routes::public::design_garment_mockup(&state, &design, side, 768).await?;
    state.store.put(&photo_key, Bytes::from(photo), "image/jpeg").await?;
    state.store.put(&garment_key, Bytes::from(garment), "image/png").await?;

    let mut tx = state.pool.begin().await?;
    let expires_at = db::tryon::insert(
        &mut *tx,
        &db::tryon::NewTryOn {
            id,
            device_id: &device,
            photo_key: &photo_key,
            garment_key: &garment_key,
            provider: state.tryon.name(),
            cache_key: &cache_key,
            ttl_hours: t.ttl_hours,
        },
    )
    .await?;
    db::jobs::enqueue(&mut *tx, db::jobs::TRYON_GENERATE, &serde_json::json!({ "tryon_id": id }), 0, 3).await?;
    tx.commit().await?;
    state.job_notify.notify_one();

    Ok((
        StatusCode::ACCEPTED,
        Json(TryOnDto { id: id.to_string(), status: TryOnState::Queued, result_url: None, error: None, expires_at: db::ts(expires_at) }),
    ))
}

pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<TryOnDto>> {
    let row = db::tryon::get(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    Ok(Json(dto(&row)))
}

pub async fn result(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Response> {
    let row = db::tryon::get(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    if row.status != "done" || row.expires_at <= chrono::Utc::now() {
        return Err(AppError::not_found());
    }
    let key = row.result_key.ok_or_else(AppError::not_found)?;
    let bytes = state.store.get(&key).await?.ok_or_else(AppError::not_found)?;
    let mime = if row.result_mime.as_deref() == Some("image/png") { "image/png" } else { "image/jpeg" };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            // A person's photo: never store it in shared or disk caches.
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

/// "Delete now": remove the photo and the result immediately.
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<DeletedDto>> {
    delete_files(&state, id).await?;
    Ok(Json(DeletedDto { deleted: true }))
}

/// Shared with the expiry sweep. Idempotent.
pub async fn delete_files(state: &AppState, id: Uuid) -> AppResult<()> {
    for key in db::tryon::mark_deleted(&state.pool, id).await? {
        state.store.delete(&key).await?;
    }
    // The result may have been written after the row was read; remove by
    // prefix as well so nothing is left behind.
    for key in state.store.list(&format!("tryon/{id}/")).await? {
        state.store.delete(&key).await?;
    }
    Ok(())
}
