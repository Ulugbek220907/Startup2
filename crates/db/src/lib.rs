//! Database access.
//!
//! Queries use sqlx's `query!` macros, which check SQL and column types
//! against a real database **at compile time**. A normal build does not
//! need a database: the checked metadata is committed in `.sqlx/` (refresh
//! it with `cargo sqlx prepare --workspace` after changing a query; CI
//! verifies it is up to date). Filtered list queries use `QueryBuilder`,
//! which still binds every value as a parameter.

pub mod auth;
pub mod catalog;
pub mod designs;
pub mod jobs;
pub mod misc;
pub mod orders;
pub mod tryon;

use chrono::{DateTime, SecondsFormat, Utc};
pub use sqlx::postgres::PgPool;
pub use sqlx::{Error, PgConnection, Postgres, Transaction};

pub type Result<T> = std::result::Result<T, sqlx::Error>;

/// Connect with a small pool; one backend process serves the whole app.
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool> {
    sqlx::postgres::PgPoolOptions::new().max_connections(max_connections).acquire_timeout(std::time::Duration::from_secs(10)).connect(database_url).await
}

/// Apply pending migrations (embedded in the binary at compile time).
pub async fn migrate(pool: &PgPool) -> std::result::Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(pool).await
}

pub async fn ping(pool: &PgPool) -> Result<()> {
    sqlx::query_scalar!("SELECT 1 AS \"one!\"").fetch_one(pool).await.map(|_| ())
}

/// RFC 3339 in UTC with second precision, the format used in all API JSON.
pub fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// True if the error is a unique-constraint violation (Postgres 23505).
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.code().as_deref() == Some("23505"))
}
pub use sqlx;
