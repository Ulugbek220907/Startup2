//! Configuration, read once at startup from environment variables.
//!
//! Every variable is documented in `.env.example`. Secrets only ever come
//! from the environment; `Config` deliberately does not implement `Debug`
//! so it cannot be dumped into logs by accident.

use std::net::SocketAddr;
use std::time::Duration;

use domain::{Lang, Tiyin};

pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub db_max_connections: u32,
    /// Public origin of the site, e.g. `https://qiyiqdoppi.uz` (no trailing slash).
    pub public_base_url: String,
    pub brand: String,
    /// Directory with the built web app (`trunk build` output).
    pub static_dir: String,
    pub storage: StorageConfig,
    pub admin_bootstrap: Option<(String, String)>,
    pub session_ttl_hours: i32,
    pub cookie_secure: bool,
    /// Extra origins allowed to call the API (the Capacitor apps).
    pub cors_allowed_origins: Vec<String>,
    /// Trust `X-Forwarded-For` from the reverse proxy for client IPs.
    pub trust_proxy: bool,
    pub max_upload_bytes: usize,
    pub allow_foil: bool,
    pub tryon: TryOnConfig,
    pub moderation_mock_flag_all: bool,
    pub moderation_mock_flag_sha256: Vec<String>,
    pub telegram: Option<TelegramConfig>,
    /// Language of admin alerts and the production sheet.
    pub admin_lang: Lang,
    pub workers: usize,
    pub draft_ttl_days: i32,
    pub log_json: bool,
}

pub enum StorageConfig {
    S3(storage::S3Config),
    Fs(String),
    Memory,
}

pub enum TryOnProviderKind {
    Mock,
    Fashn { api_key: String, base_url: String, mode: String },
}

pub struct TryOnConfig {
    pub provider: TryOnProviderKind,
    pub cost_per_image: Tiyin,
    pub ttl_hours: i32,
    pub per_device_daily: i64,
    pub global_daily: i64,
    pub daily_cost_cap: Tiyin,
    pub timeout: Duration,
}

pub struct TelegramConfig {
    pub bot_token: String,
    pub bot_username: Option<String>,
    pub admin_chat_id: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("environment variable {0} is required")]
    Missing(&'static str),
    #[error("environment variable {0} has an invalid value: {1}")]
    Invalid(&'static str, String),
}

impl Config {
    pub fn from_env() -> Result<Config, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Build from any key lookup, so tests do not have to mutate the real
    /// process environment (which is shared between test threads).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        // Empty strings count as unset: docker-compose passes `VAR=` for
        // variables that are not defined in `.env`.
        let get = |k: &'static str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let req = |k: &'static str| get(k).ok_or(ConfigError::Missing(k));
        fn parse<T: std::str::FromStr>(k: &'static str, v: Option<String>, default: T) -> Result<T, ConfigError>
        where
            T::Err: std::fmt::Display,
        {
            match v {
                None => Ok(default),
                Some(s) => s.parse().map_err(|e: T::Err| ConfigError::Invalid(k, e.to_string())),
            }
        }
        let flag = |k: &'static str, default: bool| -> Result<bool, ConfigError> {
            match get(k).as_deref() {
                None => Ok(default),
                Some("1" | "true" | "yes" | "on") => Ok(true),
                Some("0" | "false" | "no" | "off") => Ok(false),
                Some(other) => Err(ConfigError::Invalid(k, format!("{other:?} is not a boolean"))),
            }
        };
        let sum = |k: &'static str, default: i64| -> Result<Tiyin, ConfigError> {
            let v: i64 = parse(k, get(k), default)?;
            if v < 0 {
                return Err(ConfigError::Invalid(k, "must not be negative".into()));
            }
            Tiyin::from_sum(v).map_err(|e| ConfigError::Invalid(k, e.to_string()))
        };

        let public_base_url = get("PUBLIC_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into()).trim_end_matches('/').to_string();
        if !(public_base_url.starts_with("http://") || public_base_url.starts_with("https://")) {
            return Err(ConfigError::Invalid("PUBLIC_BASE_URL", "must start with http:// or https://".into()));
        }

        let storage = match get("STORAGE_BACKEND").as_deref().unwrap_or("fs") {
            "s3" => StorageConfig::S3(storage::S3Config {
                endpoint: req("S3_ENDPOINT")?,
                region: get("S3_REGION").unwrap_or_else(|| "auto".into()),
                bucket: req("S3_BUCKET")?,
                access_key: req("S3_ACCESS_KEY")?,
                secret_key: req("S3_SECRET_KEY")?,
                path_style: flag("S3_PATH_STYLE", true)?,
            }),
            "fs" => StorageConfig::Fs(get("STORAGE_FS_DIR").unwrap_or_else(|| "./data/storage".into())),
            "memory" => StorageConfig::Memory,
            other => return Err(ConfigError::Invalid("STORAGE_BACKEND", format!("{other:?}: use s3, fs or memory"))),
        };

        let admin_bootstrap = match (get("ADMIN_USERNAME"), get("ADMIN_PASSWORD")) {
            (Some(u), Some(p)) => {
                if p.chars().count() < 10 {
                    return Err(ConfigError::Invalid("ADMIN_PASSWORD", "must be at least 10 characters".into()));
                }
                Some((u, p))
            }
            (None, None) => None,
            _ => return Err(ConfigError::Invalid("ADMIN_USERNAME", "set ADMIN_USERNAME and ADMIN_PASSWORD together".into())),
        };

        let cost_per_image = sum("TRYON_COST_PER_IMAGE_SUM", 1000)?;
        let provider = match get("TRYON_PROVIDER").as_deref().unwrap_or("mock") {
            "mock" => TryOnProviderKind::Mock,
            "fashn" => TryOnProviderKind::Fashn {
                api_key: req("FASHN_API_KEY")?,
                base_url: get("FASHN_BASE_URL").unwrap_or_else(|| "https://api.fashn.ai".into()),
                mode: match get("FASHN_MODE").as_deref().unwrap_or("balanced") {
                    m @ ("performance" | "balanced" | "quality") => m.to_string(),
                    other => return Err(ConfigError::Invalid("FASHN_MODE", format!("{other:?}: use performance, balanced or quality"))),
                },
            },
            other => return Err(ConfigError::Invalid("TRYON_PROVIDER", format!("{other:?}: use mock or fashn"))),
        };

        let telegram = match get("TELEGRAM_BOT_TOKEN") {
            None => None,
            Some(bot_token) => Some(TelegramConfig {
                bot_token,
                bot_username: get("TELEGRAM_BOT_USERNAME").map(|u| u.trim_start_matches('@').to_string()),
                admin_chat_id: match get("TELEGRAM_ADMIN_CHAT_ID") {
                    None => None,
                    Some(v) => Some(v.parse().map_err(|_| ConfigError::Invalid("TELEGRAM_ADMIN_CHAT_ID", "must be a number".into()))?),
                },
            }),
        };

        let admin_lang = match get("ADMIN_LANG") {
            None => Lang::Uz,
            Some(v) => Lang::parse(&v).ok_or_else(|| ConfigError::Invalid("ADMIN_LANG", "use uz, ru or en".into()))?,
        };

        let max_upload_mb: usize = parse("MAX_UPLOAD_MB", get("MAX_UPLOAD_MB"), 10)?;
        if !(1..=50).contains(&max_upload_mb) {
            return Err(ConfigError::Invalid("MAX_UPLOAD_MB", "must be between 1 and 50".into()));
        }

        Ok(Config {
            bind_addr: parse("BIND_ADDR", get("BIND_ADDR"), SocketAddr::from(([0, 0, 0, 0], 8080)))?,
            database_url: req("DATABASE_URL")?,
            db_max_connections: parse("DB_MAX_CONNECTIONS", get("DB_MAX_CONNECTIONS"), 10)?,
            cookie_secure: flag("COOKIE_SECURE", public_base_url.starts_with("https://"))?,
            public_base_url,
            brand: get("BRAND_NAME").unwrap_or_else(|| "QiyiqDoʻppi".into()),
            static_dir: get("STATIC_DIR").unwrap_or_else(|| "./crates/web/dist".into()),
            storage,
            admin_bootstrap,
            session_ttl_hours: parse("SESSION_TTL_HOURS", get("SESSION_TTL_HOURS"), 12)?,
            cors_allowed_origins: get("CORS_ALLOWED_ORIGINS")
                .map(|v| v.split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
            trust_proxy: flag("TRUST_PROXY", false)?,
            max_upload_bytes: max_upload_mb * 1024 * 1024,
            allow_foil: flag("ALLOW_FOIL", false)?,
            tryon: TryOnConfig {
                provider,
                cost_per_image,
                ttl_hours: parse("TRYON_TTL_HOURS", get("TRYON_TTL_HOURS"), 24)?,
                per_device_daily: parse("TRYON_PER_DEVICE_DAILY", get("TRYON_PER_DEVICE_DAILY"), 5)?,
                global_daily: parse("TRYON_GLOBAL_DAILY", get("TRYON_GLOBAL_DAILY"), 300)?,
                daily_cost_cap: sum("TRYON_DAILY_COST_CAP_SUM", 200_000)?,
                timeout: Duration::from_secs(parse("TRYON_TIMEOUT_SECS", get("TRYON_TIMEOUT_SECS"), 120)?),
            },
            moderation_mock_flag_all: match get("MODERATION_MOCK_MODE").as_deref().unwrap_or("allow_all") {
                "allow_all" => false,
                "flag_all" => true,
                other => return Err(ConfigError::Invalid("MODERATION_MOCK_MODE", format!("{other:?}: use allow_all or flag_all"))),
            },
            moderation_mock_flag_sha256: get("MODERATION_MOCK_FLAG_SHA256")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
            telegram,
            admin_lang,
            workers: parse("WORKERS", get("WORKERS"), 2usize)?.clamp(1, 16),
            draft_ttl_days: parse("DRAFT_TTL_DAYS", get("DRAFT_TTL_DAYS"), 90)?,
            log_json: get("LOG_FORMAT").as_deref() != Some("pretty"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    #[test]
    fn minimal_config_uses_safe_defaults() {
        let c = cfg(&[("DATABASE_URL", "postgres://x")]).unwrap();
        assert!(matches!(c.tryon.provider, TryOnProviderKind::Mock));
        assert!(matches!(c.storage, StorageConfig::Fs(_)));
        assert!(c.telegram.is_none());
        assert!(!c.cookie_secure, "http base URL must not force Secure cookies");
        assert!(!c.allow_foil);
        assert_eq!(c.tryon.ttl_hours, 24);
        assert_eq!(c.tryon.cost_per_image, Tiyin(100_000));
        assert_eq!(c.max_upload_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn https_turns_on_secure_cookies() {
        let c = cfg(&[("DATABASE_URL", "x"), ("PUBLIC_BASE_URL", "https://example.uz/")]).unwrap();
        assert!(c.cookie_secure);
        assert_eq!(c.public_base_url, "https://example.uz");
    }

    #[test]
    fn validation_errors() {
        assert!(matches!(cfg(&[]), Err(ConfigError::Missing("DATABASE_URL"))));
        assert!(matches!(cfg(&[("DATABASE_URL", "x"), ("TRYON_PROVIDER", "fashn")]), Err(ConfigError::Missing("FASHN_API_KEY"))));
        assert!(cfg(&[("DATABASE_URL", "x"), ("ADMIN_USERNAME", "a"), ("ADMIN_PASSWORD", "short")]).is_err());
        assert!(cfg(&[("DATABASE_URL", "x"), ("ADMIN_USERNAME", "a")]).is_err());
        assert!(cfg(&[("DATABASE_URL", "x"), ("STORAGE_BACKEND", "s3")]).is_err());
        assert!(cfg(&[("DATABASE_URL", "x"), ("TRUST_PROXY", "maybe")]).is_err());
        assert!(cfg(&[("DATABASE_URL", "x"), ("PUBLIC_BASE_URL", "example.uz")]).is_err());
    }

    #[test]
    fn empty_values_count_as_unset() {
        let c = cfg(&[("DATABASE_URL", "x"), ("TELEGRAM_BOT_TOKEN", ""), ("TRYON_PROVIDER", " ")]).unwrap();
        assert!(c.telegram.is_none());
    }
}
