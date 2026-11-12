//! QiyiqDoʻppi backend: one binary that serves the API and the web app,
//! runs the background workers and (when configured) the Telegram bot.

use std::net::SocketAddr;
use std::time::Duration;

use server::config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env().map_err(|e| anyhow::anyhow!("configuration error: {e}"))?;

    // Structured JSON logs in production, readable text in development.
    // `RUST_LOG` overrides the level, e.g. RUST_LOG=server=debug,info.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if cfg.log_json {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    // In docker-compose the database may still be starting: retry briefly.
    let pool = {
        let mut attempt = 0;
        loop {
            match db::connect(&cfg.database_url, cfg.db_max_connections).await {
                Ok(pool) => break pool,
                Err(e) if attempt < 15 => {
                    attempt += 1;
                    tracing::warn!(attempt, error = %e, "database not ready, retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    };
    db::migrate(&pool).await?;

    let bind_addr = cfg.bind_addr;
    let state = server::build_state(cfg, pool)?;
    server::bootstrap_admin(&state).await?;
    tracing::info!(
        tryon = state.tryon.name(),
        messenger = state.messenger.name(),
        moderation = state.moderator.name(),
        sms = state.sms.name(),
        delivery = state.delivery.name(),
        background_removal = state.bg.name(),
        "providers selected"
    );
    if !std::path::Path::new(&state.cfg.static_dir).join("index.html").exists() {
        tracing::warn!(dir = %state.cfg.static_dir, "web app build not found; serving the API only");
    }

    server::worker::spawn(state.clone());
    if let Some(tg) = &state.cfg.telegram {
        tokio::spawn(server::bot::run(state.clone(), tg.bot_token.clone()));
    }

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(addr = %bind_addr, "listening");
    // `into_make_service_with_connect_info` exposes the peer address to
    // handlers (used for rate limiting).
    axum::serve(listener, server::build_app(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("stopped");
    Ok(())
}

/// Resolve on Ctrl-C or SIGTERM (what `docker stop` sends), so in-flight
/// requests can finish before the process exits.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
