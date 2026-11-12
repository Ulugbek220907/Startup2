//! Rate limiting counters.

use sqlx::PgPool;

use crate::Result;

/// Count one hit for `key` in the current fixed window and report whether
/// the caller is still within `limit`.
///
/// Stored in Postgres rather than in memory so limits survive restarts and
/// would keep working if a second backend instance were ever added. One
/// upsert per limited request is cheap at this app's scale.
pub async fn rate_limit_hit(pool: &PgPool, key: &str, window_secs: i32, limit: i32) -> Result<bool> {
    let count = sqlx::query_scalar!(
        "INSERT INTO rate_limits (key, window_start)
         VALUES ($1, to_timestamp(floor(extract(epoch FROM now()) / $2::float8) * $2::float8))
         ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1
         RETURNING count",
        key,
        window_secs as f64
    )
    .fetch_one(pool)
    .await?;
    Ok(count <= limit)
}

pub async fn purge_rate_limits(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query!("DELETE FROM rate_limits WHERE window_start < now() - interval '2 days'").execute(pool).await?.rows_affected())
}
