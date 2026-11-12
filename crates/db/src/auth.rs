//! Admin accounts, sessions and the audit log.

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;

pub struct AdminRow {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
}

pub async fn admin_by_username(pool: &PgPool, username: &str) -> Result<Option<AdminRow>> {
    sqlx::query_as!(AdminRow, "SELECT id, username, password_hash FROM admins WHERE username = $1", username).fetch_optional(pool).await
}

pub async fn count_admins(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM admins"#).fetch_one(pool).await
}

/// Create the admin or replace its password hash.
pub async fn upsert_admin(pool: &PgPool, username: &str, password_hash: &str) -> Result<()> {
    sqlx::query!(
        "INSERT INTO admins (username, password_hash) VALUES ($1, $2)
         ON CONFLICT (username) DO UPDATE SET password_hash = EXCLUDED.password_hash",
        username,
        password_hash
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub struct SessionRow {
    pub admin_id: Uuid,
    pub username: String,
    pub csrf_token: String,
}

pub async fn insert_session(pool: &PgPool, admin_id: Uuid, token_hash: &str, csrf_token: &str, ttl_hours: i32) -> Result<()> {
    sqlx::query!(
        "INSERT INTO sessions (admin_id, token_hash, csrf_token, expires_at) VALUES ($1, $2, $3, now() + make_interval(hours => $4))",
        admin_id,
        token_hash,
        csrf_token,
        ttl_hours
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn session(pool: &PgPool, token_hash: &str) -> Result<Option<SessionRow>> {
    sqlx::query_as!(
        SessionRow,
        "SELECT s.admin_id, a.username, s.csrf_token FROM sessions s JOIN admins a ON a.id = s.admin_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
        token_hash
    )
    .fetch_optional(pool)
    .await
}

pub async fn delete_session(pool: &PgPool, token_hash: &str) -> Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE token_hash = $1", token_hash).execute(pool).await?;
    Ok(())
}

pub async fn purge_sessions(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query!("DELETE FROM sessions WHERE expires_at <= now()").execute(pool).await?.rows_affected())
}

pub async fn audit(
    ex: impl PgExecutor<'_>,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: Option<&str>,
    detail: &serde_json::Value,
    ip: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        "INSERT INTO audit_log (actor, action, entity, entity_id, detail, ip) VALUES ($1, $2, $3, $4, $5, $6)",
        actor,
        action,
        entity,
        entity_id,
        detail,
        ip
    )
    .execute(ex)
    .await?;
    Ok(())
}

pub struct AuditRow {
    pub created_at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub detail: serde_json::Value,
}

pub async fn audit_recent(pool: &PgPool, limit: i64) -> Result<Vec<AuditRow>> {
    sqlx::query_as!(AuditRow, "SELECT created_at, actor, action, entity, entity_id, detail FROM audit_log ORDER BY created_at DESC, id DESC LIMIT $1", limit)
        .fetch_all(pool)
        .await
}
