//! Uploaded assets, designs, moderation and reports.

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;

pub struct AssetRow {
    pub id: Uuid,
    pub mime: String,
    pub width: i32,
    pub height: i32,
    pub storage_key: String,
    pub preview_key: String,
}

pub struct NewAsset<'a> {
    pub id: Uuid,
    pub sha256: &'a str,
    pub mime: &'a str,
    pub width: i32,
    pub height: i32,
    pub bytes: i64,
    pub storage_key: &'a str,
    pub preview_key: &'a str,
}

pub async fn insert_asset(pool: &PgPool, a: &NewAsset<'_>) -> Result<()> {
    sqlx::query!(
        "INSERT INTO assets (id, sha256, mime, width, height, bytes, storage_key, preview_key) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        a.id,
        a.sha256,
        a.mime,
        a.width,
        a.height,
        a.bytes,
        a.storage_key,
        a.preview_key
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn asset(pool: &PgPool, id: Uuid) -> Result<Option<AssetRow>> {
    sqlx::query_as!(AssetRow, "SELECT id, mime, width, height, storage_key, preview_key FROM assets WHERE id = $1", id).fetch_optional(pool).await
}

pub async fn assets_by_ids(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<AssetRow>> {
    sqlx::query_as!(AssetRow, "SELECT id, mime, width, height, storage_key, preview_key FROM assets WHERE id = ANY($1)", ids).fetch_all(pool).await
}

pub struct DesignRow {
    pub id: Uuid,
    pub slug: String,
    pub edit_token_hash: String,
    pub doc: serde_json::Value,
    pub garment_slug: String,
    pub moderation_status: String,
    pub moderation_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_design(
    ex: impl PgExecutor<'_>,
    slug: &str,
    edit_token_hash: &str,
    schema_version: i32,
    doc: &serde_json::Value,
    garment_slug: &str,
) -> Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO designs (slug, edit_token_hash, schema_version, doc, garment_slug) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        slug,
        edit_token_hash,
        schema_version,
        doc,
        garment_slug
    )
    .fetch_one(ex)
    .await
}

pub async fn design_by_slug(pool: &PgPool, slug: &str) -> Result<Option<DesignRow>> {
    sqlx::query_as!(
        DesignRow,
        "SELECT id, slug, edit_token_hash, doc, garment_slug, moderation_status, moderation_reason, created_at FROM designs WHERE slug = $1",
        slug
    )
    .fetch_optional(pool)
    .await
}

pub async fn design_by_id(pool: &PgPool, id: Uuid) -> Result<Option<DesignRow>> {
    sqlx::query_as!(
        DesignRow,
        "SELECT id, slug, edit_token_hash, doc, garment_slug, moderation_status, moderation_reason, created_at FROM designs WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await
}

/// Update a design's document. The edit token is part of the WHERE clause,
/// so a wrong token updates nothing. Editing resets moderation.
pub async fn update_design(pool: &PgPool, slug: &str, edit_token_hash: &str, doc: &serde_json::Value, garment_slug: &str) -> Result<bool> {
    let r = sqlx::query!(
        "UPDATE designs SET doc = $3, garment_slug = $4, updated_at = now(),
                moderation_status = CASE WHEN moderation_status = 'rejected' THEN 'rejected' ELSE 'pending' END
         WHERE slug = $1 AND edit_token_hash = $2",
        slug,
        edit_token_hash,
        doc,
        garment_slug
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Set the result of the automatic checks. Never overrides a human decision
/// (approved/rejected) and never clears an existing flag.
pub async fn set_auto_moderation(ex: impl PgExecutor<'_>, id: Uuid, flagged: bool, reason: Option<&str>) -> Result<()> {
    sqlx::query!(
        "UPDATE designs SET
            moderation_status = CASE
                WHEN moderation_status IN ('approved', 'rejected', 'flagged') THEN moderation_status
                WHEN $2 THEN 'flagged' ELSE 'clean' END,
            moderation_reason = CASE
                WHEN moderation_status IN ('approved', 'rejected', 'flagged') THEN moderation_reason
                ELSE $3 END
         WHERE id = $1",
        id,
        flagged,
        reason
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// Flag a design for review (reports), unless a human already rejected it.
pub async fn flag_design(ex: impl PgExecutor<'_>, id: Uuid, reason: &str) -> Result<()> {
    sqlx::query!("UPDATE designs SET moderation_status = 'flagged', moderation_reason = $2 WHERE id = $1 AND moderation_status <> 'rejected'", id, reason)
        .execute(ex)
        .await?;
    Ok(())
}

/// Record a human decision.
pub async fn decide_moderation(ex: impl PgExecutor<'_>, id: Uuid, approve: bool, note: Option<&str>) -> Result<bool> {
    let status = if approve { "approved" } else { "rejected" };
    let r = sqlx::query!("UPDATE designs SET moderation_status = $2, moderation_reason = COALESCE($3, moderation_reason) WHERE id = $1", id, status, note)
        .execute(ex)
        .await?;
    Ok(r.rows_affected() == 1)
}

pub struct ModerationRow {
    pub id: Uuid,
    pub slug: String,
    pub doc: serde_json::Value,
    pub moderation_status: String,
    pub moderation_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub order_id: Option<Uuid>,
    pub order_code: Option<String>,
}

/// Flagged designs, newest first, with the order that uses them (if any).
pub async fn moderation_queue(pool: &PgPool) -> Result<Vec<ModerationRow>> {
    sqlx::query_as!(
        ModerationRow,
        r#"SELECT d.id, d.slug, d.doc, d.moderation_status, d.moderation_reason, d.created_at,
                  o.id AS "order_id?", o.code AS "order_code?"
           FROM designs d
           LEFT JOIN LATERAL (
               SELECT o.id, o.code FROM order_items i JOIN orders o ON o.id = i.order_id
               WHERE i.design_id = d.id ORDER BY o.created_at DESC LIMIT 1
           ) o ON TRUE
           WHERE d.moderation_status = 'flagged'
           ORDER BY d.updated_at DESC LIMIT 200"#
    )
    .fetch_all(pool)
    .await
}

pub async fn insert_report(ex: impl PgExecutor<'_>, design_id: Uuid, kind: &str, reason: &str, contact: Option<&str>) -> Result<Uuid> {
    sqlx::query_scalar!("INSERT INTO design_reports (design_id, kind, reason, contact) VALUES ($1, $2, $3, $4) RETURNING id", design_id, kind, reason, contact)
        .fetch_one(ex)
        .await
}

pub struct ReportRow {
    pub id: Uuid,
    pub design_id: Uuid,
    pub design_slug: String,
    pub kind: String,
    pub reason: String,
    pub contact: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

pub async fn reports(pool: &PgPool, only_open: bool) -> Result<Vec<ReportRow>> {
    sqlx::query_as!(
        ReportRow,
        "SELECT r.id, r.design_id, d.slug AS design_slug, r.kind, r.reason, r.contact, r.status, r.created_at
         FROM design_reports r JOIN designs d ON d.id = r.design_id
         WHERE ($1 = FALSE OR r.status = 'open')
         ORDER BY r.created_at DESC LIMIT 200",
        only_open
    )
    .fetch_all(pool)
    .await
}

/// Close a report; returns the design id when the report was open.
pub async fn resolve_report(ex: impl PgExecutor<'_>, id: Uuid, status: &str, actor: &str) -> Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "UPDATE design_reports SET status = $2, resolved_at = now(), resolved_by = $3 WHERE id = $1 AND status = 'open' RETURNING design_id",
        id,
        status,
        actor
    )
    .fetch_optional(ex)
    .await
}

/// Delete drafts that were never ordered and not touched for `days` days.
/// Returns the number of designs removed.
pub async fn purge_stale_drafts(pool: &PgPool, days: i32) -> Result<u64> {
    let r = sqlx::query!(
        "DELETE FROM designs d
         WHERE d.updated_at < now() - make_interval(days => $1)
           AND NOT EXISTS (SELECT 1 FROM order_items i WHERE i.design_id = d.id)
           AND NOT EXISTS (SELECT 1 FROM design_reports r WHERE r.design_id = d.id AND r.status = 'open')",
        days
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// Uploaded assets older than `days` days that no stored design or order
/// references any more. Returned so the caller can delete the files first.
pub async fn orphan_assets(pool: &PgPool, days: i32) -> Result<Vec<AssetRow>> {
    sqlx::query_as!(
        AssetRow,
        "SELECT a.id, a.mime, a.width, a.height, a.storage_key, a.preview_key FROM assets a
         WHERE a.created_at < now() - make_interval(days => $1)
           AND NOT EXISTS (SELECT 1 FROM designs d WHERE d.doc::text LIKE '%' || a.id::text || '%')
           AND NOT EXISTS (SELECT 1 FROM order_items i WHERE i.design_doc::text LIKE '%' || a.id::text || '%')
         LIMIT 500",
        days
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_asset(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query!("DELETE FROM assets WHERE id = $1", id).execute(pool).await?;
    Ok(())
}
