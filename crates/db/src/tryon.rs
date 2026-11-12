//! Photo try-on jobs (Tier B).

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;

pub struct TryOnRow {
    pub id: Uuid,
    pub device_id: String,
    pub photo_key: Option<String>,
    pub garment_key: Option<String>,
    pub result_key: Option<String>,
    pub result_mime: Option<String>,
    pub provider: String,
    pub status: String,
    pub error: Option<String>,
    pub cache_key: String,
    pub expires_at: DateTime<Utc>,
}

pub struct NewTryOn<'a> {
    pub id: Uuid,
    pub device_id: &'a str,
    pub photo_key: &'a str,
    pub garment_key: &'a str,
    pub provider: &'a str,
    pub cache_key: &'a str,
    pub ttl_hours: i32,
}

pub async fn insert(ex: impl PgExecutor<'_>, t: &NewTryOn<'_>) -> Result<DateTime<Utc>> {
    sqlx::query_scalar!(
        "INSERT INTO tryon_jobs (id, device_id, photo_key, garment_key, provider, cache_key, consent_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + make_interval(hours => $7)) RETURNING expires_at",
        t.id,
        t.device_id,
        t.photo_key,
        t.garment_key,
        t.provider,
        t.cache_key,
        t.ttl_hours
    )
    .fetch_one(ex)
    .await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<TryOnRow>> {
    sqlx::query_as!(
        TryOnRow,
        "SELECT id, device_id, photo_key, garment_key, result_key, result_mime, provider, status, error, cache_key, expires_at
         FROM tryon_jobs WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await
}

/// A finished, unexpired result for the same photo + garment, if any.
pub async fn find_cached(pool: &PgPool, cache_key: &str, device_id: &str) -> Result<Option<TryOnRow>> {
    sqlx::query_as!(
        TryOnRow,
        "SELECT id, device_id, photo_key, garment_key, result_key, result_mime, provider, status, error, cache_key, expires_at
         FROM tryon_jobs
         WHERE cache_key = $1 AND device_id = $2 AND status = 'done' AND expires_at > now()
         ORDER BY created_at DESC LIMIT 1",
        cache_key,
        device_id
    )
    .fetch_optional(pool)
    .await
}

/// Mark as running; false if the job was deleted or already handled.
pub async fn set_running(pool: &PgPool, id: Uuid) -> Result<bool> {
    let r = sqlx::query!("UPDATE tryon_jobs SET status = 'running' WHERE id = $1 AND status IN ('queued', 'running')", id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

/// Store the result; false if the customer deleted the job meanwhile (the
/// caller must then delete the result file it just wrote).
pub async fn set_done(pool: &PgPool, id: Uuid, result_key: &str, mime: &str, cost_tiyin: i64) -> Result<bool> {
    let r = sqlx::query!(
        "UPDATE tryon_jobs SET status = 'done', result_key = $2, result_mime = $3, cost_tiyin = $4, finished_at = now()
         WHERE id = $1 AND status = 'running'",
        id,
        result_key,
        mime,
        cost_tiyin
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn set_failed(pool: &PgPool, id: Uuid, error: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE tryon_jobs SET status = 'failed', error = $2, finished_at = now() WHERE id = $1 AND status IN ('queued', 'running')",
        id,
        error
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark deleted and forget the file keys. Returns the keys that must be
/// removed from storage.
pub async fn mark_deleted(pool: &PgPool, id: Uuid) -> Result<Vec<String>> {
    let row = sqlx::query!(
        "UPDATE tryon_jobs t SET status = 'deleted', photo_key = NULL, garment_key = NULL, result_key = NULL
         FROM (SELECT id, photo_key, garment_key, result_key FROM tryon_jobs WHERE id = $1 FOR UPDATE) old
         WHERE t.id = old.id
         RETURNING old.photo_key, old.garment_key, old.result_key",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| [r.photo_key, r.garment_key, r.result_key].into_iter().flatten().collect()).unwrap_or_default())
}

/// Ids of jobs past their TTL that still hold files.
pub async fn expired(pool: &PgPool, limit: i64) -> Result<Vec<Uuid>> {
    sqlx::query_scalar!("SELECT id FROM tryon_jobs WHERE status <> 'deleted' AND expires_at <= now() ORDER BY expires_at LIMIT $1", limit)
        .fetch_all(pool)
        .await
}

/// Jobs started by a device in the last 24 hours.
pub async fn count_recent_for_device(pool: &PgPool, device_id: &str) -> Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM tryon_jobs WHERE device_id = $1 AND created_at > now() - interval '24 hours'"#,
        device_id
    )
    .fetch_one(pool)
    .await
}

/// Number of jobs and money spent on generation since midnight UTC, for the
/// global limit and the daily cost cap. Queued/running jobs are counted at
/// `pending_cost` each so the cap cannot be overshot by parallel requests.
pub async fn usage_today(pool: &PgPool, pending_cost: i64) -> Result<(i64, i64)> {
    let r = sqlx::query!(
        r#"SELECT count(*) AS "n!",
                  COALESCE(sum(CASE WHEN status IN ('queued', 'running') THEN $1::bigint ELSE cost_tiyin END), 0)::bigint AS "cost!"
           FROM tryon_jobs WHERE created_at >= date_trunc('day', now())"#,
        pending_cost
    )
    .fetch_one(pool)
    .await?;
    Ok((r.n, r.cost))
}
