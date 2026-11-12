//! Postgres-backed job queue.
//!
//! No broker needed: a worker claims a row with `FOR UPDATE SKIP LOCKED`
//! (other workers skip rows that are being claimed instead of waiting),
//! marks it running with a lease, and deletes the lease on completion. A
//! worker that dies leaves an expired lease, and the job is picked up again.
//! Jobs are therefore **at-least-once**: handlers must be idempotent.

use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;

pub const RENDER_MOCKUP: &str = "render_mockup";
pub const RENDER_ORDER_FILES: &str = "render_order_files";
pub const TRYON_GENERATE: &str = "tryon_generate";
pub const MODERATE_DESIGN: &str = "moderate_design";
pub const NOTIFY_ADMIN: &str = "notify_admin";
pub const NOTIFY_CUSTOMER: &str = "notify_customer";
pub const PURGE_EXPIRED: &str = "purge_expired";

pub struct Job {
    pub id: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
    pub max_attempts: i32,
}

/// Queue a job. Pass a transaction to enqueue atomically with other writes.
pub async fn enqueue(ex: impl PgExecutor<'_>, kind: &str, payload: &serde_json::Value, delay_secs: i32, max_attempts: i32) -> Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO jobs (kind, payload, run_at, max_attempts) VALUES ($1, $2, now() + make_interval(secs => $3), $4) RETURNING public_id",
        kind,
        payload,
        delay_secs as f64,
        max_attempts
    )
    .fetch_one(ex)
    .await
}

/// Claim the next due job, or a running job whose lease expired.
pub async fn claim(pool: &PgPool, lease_secs: i32) -> Result<Option<Job>> {
    sqlx::query_as!(
        Job,
        "UPDATE jobs SET status = 'running', attempts = attempts + 1, locked_until = now() + make_interval(secs => $1)
         WHERE id = (
             SELECT id FROM jobs
             WHERE (status = 'queued' AND run_at <= now()) OR (status = 'running' AND locked_until < now())
             ORDER BY run_at
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING id, kind, payload, attempts, max_attempts",
        lease_secs as f64
    )
    .fetch_optional(pool)
    .await
}

pub async fn complete(pool: &PgPool, id: i64) -> Result<()> {
    sqlx::query!("UPDATE jobs SET status = 'done', locked_until = NULL, finished_at = now(), last_error = NULL WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Record a failure. Retries with exponential backoff (30 s, 2 min, 8 min,
/// ...) until `max_attempts`, then marks the job failed. Returns true when
/// the job was given up on.
pub async fn fail(pool: &PgPool, job: &Job, error: &str, retryable: bool) -> Result<bool> {
    let give_up = !retryable || job.attempts >= job.max_attempts;
    // Keep stored errors short; they may be shown in the admin UI.
    let error: String = error.chars().take(500).collect();
    if give_up {
        sqlx::query!("UPDATE jobs SET status = 'failed', locked_until = NULL, finished_at = now(), last_error = $2 WHERE id = $1", job.id, error)
            .execute(pool)
            .await?;
    } else {
        let backoff = 30.0 * 4f64.powi((job.attempts - 1).clamp(0, 6));
        sqlx::query!(
            "UPDATE jobs SET status = 'queued', locked_until = NULL, last_error = $2, run_at = now() + make_interval(secs => $3) WHERE id = $1",
            job.id,
            error,
            backoff
        )
        .execute(pool)
        .await?;
    }
    Ok(give_up)
}

/// Status of a job by its public id, for client polling.
pub async fn status(pool: &PgPool, public_id: Uuid) -> Result<Option<String>> {
    sqlx::query_scalar!("SELECT status FROM jobs WHERE public_id = $1", public_id).fetch_optional(pool).await
}

/// True if a queued or running job of this kind exists.
pub async fn pending_exists(pool: &PgPool, kind: &str) -> Result<bool> {
    sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM jobs WHERE kind = $1 AND status IN ('queued', 'running')) AS "e!""#, kind)
        .fetch_one(pool)
        .await
}

/// Remove finished jobs older than `days` days so the table stays small.
pub async fn purge_finished(pool: &PgPool, days: i32) -> Result<u64> {
    let r = sqlx::query!(
        "DELETE FROM jobs WHERE status IN ('done', 'failed') AND finished_at < now() - make_interval(days => $1)",
        days
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// Queue depth by status, for the metrics endpoint.
pub async fn counts(pool: &PgPool) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query!(r#"SELECT status, count(*) AS "n!" FROM jobs GROUP BY status"#).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| (r.status, r.n)).collect())
}
