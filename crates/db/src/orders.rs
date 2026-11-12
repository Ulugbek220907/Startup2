//! Customers and orders.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool, QueryBuilder};
use uuid::Uuid;

use crate::Result;

// ----------------------------------------------------------- customers

pub struct CustomerRow {
    pub id: Uuid,
    pub phone: Option<String>,
    pub telegram_id: Option<i64>,
    pub telegram_chat_id: Option<i64>,
    pub name: Option<String>,
    pub lang: String,
}

/// Find or create the customer for a phone number. An existing customer's
/// name and language are refreshed from the latest order.
pub async fn upsert_customer_by_phone(conn: &mut PgConnection, phone: &str, name: Option<&str>, lang: &str) -> Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO customers (phone, name, lang) VALUES ($1, $2, $3)
         ON CONFLICT (phone) DO UPDATE SET name = COALESCE(EXCLUDED.name, customers.name), lang = EXCLUDED.lang
         RETURNING id",
        phone,
        name,
        lang
    )
    .fetch_one(conn)
    .await
}

pub async fn customer_by_phone(pool: &PgPool, phone: &str) -> Result<Option<CustomerRow>> {
    sqlx::query_as!(CustomerRow, "SELECT id, phone, telegram_id, telegram_chat_id, name, lang FROM customers WHERE phone = $1", phone)
        .fetch_optional(pool)
        .await
}

pub async fn customer_by_telegram(pool: &PgPool, telegram_id: i64) -> Result<Option<CustomerRow>> {
    sqlx::query_as!(CustomerRow, "SELECT id, phone, telegram_id, telegram_chat_id, name, lang FROM customers WHERE telegram_id = $1", telegram_id)
        .fetch_optional(pool)
        .await
}

/// Remember a Telegram user before they share a phone number.
pub async fn upsert_telegram_user(pool: &PgPool, telegram_id: i64, chat_id: Option<i64>, name: Option<&str>, lang: &str) -> Result<()> {
    sqlx::query!(
        "INSERT INTO customers (telegram_id, telegram_chat_id, name, lang) VALUES ($1, $2, $3, $4)
         ON CONFLICT (telegram_id) DO UPDATE SET
            telegram_chat_id = COALESCE(EXCLUDED.telegram_chat_id, customers.telegram_chat_id),
            name = COALESCE(customers.name, EXCLUDED.name)",
        telegram_id,
        chat_id,
        name,
        lang
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Attach a Telegram account to a phone number. If the phone already has a
/// customer row (from a guest order), the Telegram identity moves onto it so
/// order history and notifications line up.
pub async fn link_telegram_phone(conn: &mut PgConnection, telegram_id: i64, chat_id: i64, phone: &str, lang: &str) -> Result<()> {
    let by_phone = sqlx::query_scalar!("SELECT id FROM customers WHERE phone = $1", phone).fetch_optional(&mut *conn).await?;
    match by_phone {
        Some(id) => {
            // Free the unique telegram_id from any phone-less placeholder row.
            sqlx::query!("DELETE FROM customers WHERE telegram_id = $1 AND id <> $2 AND phone IS NULL", telegram_id, id).execute(&mut *conn).await?;
            sqlx::query!("UPDATE customers SET telegram_id = NULL, telegram_chat_id = NULL WHERE telegram_id = $1 AND id <> $2", telegram_id, id)
                .execute(&mut *conn)
                .await?;
            sqlx::query!("UPDATE customers SET telegram_id = $2, telegram_chat_id = $3 WHERE id = $1", id, telegram_id, chat_id).execute(&mut *conn).await?;
        }
        None => {
            sqlx::query!(
                "INSERT INTO customers (phone, telegram_id, telegram_chat_id, lang) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (telegram_id) DO UPDATE SET phone = EXCLUDED.phone, telegram_chat_id = EXCLUDED.telegram_chat_id",
                phone,
                telegram_id,
                chat_id,
                lang
            )
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

pub async fn set_customer_lang(pool: &PgPool, telegram_id: i64, lang: &str) -> Result<()> {
    sqlx::query!("UPDATE customers SET lang = $2 WHERE telegram_id = $1", telegram_id, lang).execute(pool).await?;
    Ok(())
}

// -------------------------------------------------------------- orders

pub struct NewOrder<'a> {
    pub code: &'a str,
    pub customer_id: Uuid,
    pub phone: &'a str,
    pub customer_name: Option<&'a str>,
    pub delivery_method: &'a str,
    pub region_slug: Option<&'a str>,
    pub address: Option<&'a str>,
    pub notes: Option<&'a str>,
    pub lang: &'a str,
    pub subtotal: i64,
    pub discount: i64,
    pub delivery_fee: i64,
    pub total: i64,
    pub quote: &'a serde_json::Value,
}

pub struct NewItem<'a> {
    pub design_id: Uuid,
    pub design_doc: &'a serde_json::Value,
    pub garment_slug: &'a str,
    pub garment_kind: &'a str,
    pub garment_name: &'a str,
    pub color_slug: &'a str,
    pub color_name: &'a str,
    pub color_hex: &'a str,
    pub size: &'a str,
    pub quantity: i32,
    pub finish: &'a str,
    pub unit_price: i64,
    pub areas: &'a serde_json::Value,
}

/// Insert an order with its item and the first status-log entry. Call inside
/// a transaction together with the job inserts.
pub async fn insert_order(conn: &mut PgConnection, o: &NewOrder<'_>, item: &NewItem<'_>) -> Result<Uuid> {
    let id = sqlx::query_scalar!(
        "INSERT INTO orders (code, customer_id, phone, customer_name, delivery_method, region_slug, address, notes, lang,
                             subtotal_tiyin, discount_tiyin, delivery_fee_tiyin, total_tiyin, quote)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id",
        o.code,
        o.customer_id,
        o.phone,
        o.customer_name,
        o.delivery_method,
        o.region_slug,
        o.address,
        o.notes,
        o.lang,
        o.subtotal,
        o.discount,
        o.delivery_fee,
        o.total,
        o.quote
    )
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO order_items (order_id, design_id, design_doc, garment_slug, garment_kind, garment_name, color_slug, color_name,
                                  color_hex, size, quantity, finish, unit_price_tiyin, areas)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        id,
        item.design_id,
        item.design_doc,
        item.garment_slug,
        item.garment_kind,
        item.garment_name,
        item.color_slug,
        item.color_name,
        item.color_hex,
        item.size,
        item.quantity,
        item.finish,
        item.unit_price,
        item.areas
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("INSERT INTO order_status_log (order_id, from_status, to_status, actor) VALUES ($1, NULL, 'new', 'customer')", id).execute(&mut *conn).await?;
    Ok(id)
}

/// An order joined with its (single) item and the design's moderation state.
#[derive(sqlx::FromRow)]
pub struct OrderRow {
    pub id: Uuid,
    pub code: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub customer_id: Option<Uuid>,
    pub phone: Option<String>,
    pub customer_name: Option<String>,
    pub delivery_method: String,
    pub region_slug: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub lang: String,
    pub total_tiyin: i64,
    pub quote: serde_json::Value,
    pub files_status: String,
    pub design_id: Uuid,
    pub design_doc: serde_json::Value,
    pub garment_slug: String,
    pub garment_kind: String,
    pub garment_name: String,
    pub color_slug: String,
    pub color_name: String,
    pub color_hex: String,
    pub size: String,
    pub quantity: i32,
    pub finish: String,
    pub areas: serde_json::Value,
    pub moderation_status: String,
    pub moderation_reason: Option<String>,
}

/// Shared SELECT for [`OrderRow`]; callers append WHERE/ORDER clauses.
const ORDER_SELECT: &str = "SELECT o.id, o.code, o.status, o.created_at, o.customer_id, o.phone, o.customer_name, o.delivery_method,
        o.region_slug, o.address, o.notes, o.lang, o.total_tiyin, o.quote, o.files_status,
        i.design_id, i.design_doc, i.garment_slug, i.garment_kind, i.garment_name, i.color_slug, i.color_name, i.color_hex,
        i.size, i.quantity, i.finish, i.areas, d.moderation_status, d.moderation_reason
     FROM orders o
     JOIN LATERAL (SELECT * FROM order_items WHERE order_id = o.id ORDER BY id LIMIT 1) i ON TRUE
     JOIN designs d ON d.id = i.design_id";

pub async fn order_by_id(pool: &PgPool, id: Uuid) -> Result<Option<OrderRow>> {
    let mut qb = QueryBuilder::new(ORDER_SELECT);
    // `push_bind` adds a `$n` placeholder and binds the value: user input is
    // never concatenated into the SQL text.
    qb.push(" WHERE o.id = ").push_bind(id);
    qb.build_query_as::<OrderRow>().fetch_optional(pool).await
}

pub async fn order_by_code_and_phone(pool: &PgPool, code: &str, phone: &str) -> Result<Option<OrderRow>> {
    let mut qb = QueryBuilder::new(ORDER_SELECT);
    qb.push(" WHERE o.code = ").push_bind(code.to_string()).push(" AND o.phone = ").push_bind(phone.to_string());
    qb.build_query_as::<OrderRow>().fetch_optional(pool).await
}

pub async fn orders_by_phone(pool: &PgPool, phone: &str, limit: i64) -> Result<Vec<OrderRow>> {
    let mut qb = QueryBuilder::new(ORDER_SELECT);
    qb.push(" WHERE o.phone = ").push_bind(phone.to_string()).push(" ORDER BY o.created_at DESC LIMIT ").push_bind(limit);
    qb.build_query_as::<OrderRow>().fetch_all(pool).await
}

#[derive(Default)]
pub struct OrderFilter<'a> {
    pub status: Option<&'a str>,
    /// Matches order code or phone (substring, case-insensitive).
    pub search: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

fn push_filter(qb: &mut QueryBuilder<sqlx::Postgres>, f: &OrderFilter<'_>) {
    qb.push(" WHERE TRUE");
    if let Some(s) = f.status {
        qb.push(" AND o.status = ").push_bind(s.to_string());
    }
    if let Some(q) = f.search.map(str::trim).filter(|q| !q.is_empty()) {
        // Escape LIKE wildcards so a search for "%" does not match everything.
        let pattern = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        qb.push(" AND (o.code ILIKE ").push_bind(pattern.clone()).push(" OR o.phone ILIKE ").push_bind(pattern).push(")");
    }
}

pub async fn list_orders(pool: &PgPool, f: &OrderFilter<'_>) -> Result<(Vec<OrderRow>, i64)> {
    let mut qb = QueryBuilder::new(ORDER_SELECT);
    push_filter(&mut qb, f);
    qb.push(" ORDER BY o.created_at DESC LIMIT ").push_bind(f.limit.clamp(1, 200)).push(" OFFSET ").push_bind(f.offset.max(0));
    let rows = qb.build_query_as::<OrderRow>().fetch_all(pool).await?;

    let mut count = QueryBuilder::new("SELECT count(*) FROM orders o");
    push_filter(&mut count, f);
    let total: i64 = count.build_query_scalar().fetch_one(pool).await?;
    Ok((rows, total))
}

pub struct StatusLogRow {
    pub from_status: Option<String>,
    pub to_status: String,
    pub actor: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn status_log(pool: &PgPool, order_id: Uuid) -> Result<Vec<StatusLogRow>> {
    sqlx::query_as!(
        StatusLogRow,
        "SELECT from_status, to_status, actor, note, created_at FROM order_status_log WHERE order_id = $1 ORDER BY created_at, id",
        order_id
    )
    .fetch_all(pool)
    .await
}

/// Move an order from `from` to `to` and log it. The current status is part
/// of the WHERE clause, so two admins clicking at once cannot both succeed.
/// Returns false when the order was no longer in `from`.
pub async fn change_status(conn: &mut PgConnection, order_id: Uuid, from: &str, to: &str, actor: &str, note: Option<&str>) -> Result<bool> {
    let r = sqlx::query!("UPDATE orders SET status = $3, updated_at = now() WHERE id = $1 AND status = $2", order_id, from, to).execute(&mut *conn).await?;
    if r.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query!("INSERT INTO order_status_log (order_id, from_status, to_status, actor, note) VALUES ($1, $2, $3, $4, $5)", order_id, from, to, actor, note)
        .execute(&mut *conn)
        .await?;
    Ok(true)
}

// --------------------------------------------------------------- files

pub struct FileRow {
    pub kind: String,
    pub storage_key: String,
    pub mime: String,
    pub bytes: i64,
}

pub async fn upsert_file(ex: impl PgExecutor<'_>, order_id: Uuid, kind: &str, storage_key: &str, mime: &str, bytes: i64) -> Result<()> {
    sqlx::query!(
        "INSERT INTO order_files (order_id, kind, storage_key, mime, bytes) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (order_id, kind) DO UPDATE SET storage_key = EXCLUDED.storage_key, mime = EXCLUDED.mime,
            bytes = EXCLUDED.bytes, created_at = now()",
        order_id,
        kind,
        storage_key,
        mime,
        bytes
    )
    .execute(ex)
    .await?;
    Ok(())
}

pub async fn files(pool: &PgPool, order_id: Uuid) -> Result<Vec<FileRow>> {
    sqlx::query_as!(FileRow, "SELECT kind, storage_key, mime, bytes FROM order_files WHERE order_id = $1 ORDER BY kind", order_id).fetch_all(pool).await
}

pub async fn file(pool: &PgPool, order_id: Uuid, kind: &str) -> Result<Option<FileRow>> {
    sqlx::query_as!(FileRow, "SELECT kind, storage_key, mime, bytes FROM order_files WHERE order_id = $1 AND kind = $2", order_id, kind)
        .fetch_optional(pool)
        .await
}

pub async fn set_files_status(ex: impl PgExecutor<'_>, order_id: Uuid, status: &str) -> Result<()> {
    sqlx::query!("UPDATE orders SET files_status = $2, updated_at = now() WHERE id = $1", order_id, status).execute(ex).await?;
    Ok(())
}

// ------------------------------------------------------------- privacy

pub async fn count_active_orders(pool: &PgPool, phone: &str) -> Result<i64> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM orders WHERE phone = $1 AND status NOT IN ('delivered', 'cancelled')"#, phone).fetch_one(pool).await
}

/// Erase a customer's personal data: finished orders keep their amounts and
/// design (the business record) but lose phone, name, address and notes; the
/// customer row is removed.
pub async fn anonymise_customer(conn: &mut PgConnection, phone: &str) -> Result<u64> {
    let r = sqlx::query!(
        "UPDATE orders SET phone = NULL, customer_name = NULL, address = NULL, notes = NULL, customer_id = NULL, updated_at = now()
         WHERE phone = $1",
        phone
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("DELETE FROM customers WHERE phone = $1", phone).execute(&mut *conn).await?;
    Ok(r.rows_affected())
}
