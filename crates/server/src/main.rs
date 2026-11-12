//! QiyiqDoʻppi backend: one binary that serves the API and the web app,
//! runs the background workers and (when configured) the Telegram bot.

use std::net::SocketAddr;
use std::time::Duration;

use server::config::Config;
use tracing_subscriber::EnvFilter;

/// `qiyiq-server healthcheck`: exit 0 if the local server answers
/// `/healthz`. Used by the Docker HEALTHCHECK, so the image needs no curl.
fn healthcheck() -> ! {
    use std::io::{Read, Write};
    let port = std::env::var("BIND_ADDR").ok().and_then(|a| a.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())).unwrap_or(8080);
    let ok = std::net::TcpStream::connect(("127.0.0.1", port))
        .and_then(|mut s| {
            s.set_read_timeout(Some(Duration::from_secs(3)))?;
            s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf)?;
            Ok(buf[..n].windows(3).any(|w| w == b"200"))
        })
        .unwrap_or(false);
    std::process::exit(if ok { 0 } else { 1 })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        healthcheck();
    }
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

    if std::env::var("SEED_DEMO").is_ok_and(|v| v == "true" || v == "1") {
        if let Err(e) = server::seed_demo(&state).await {
            tracing::warn!(error = %e, "demo seed failed");
        }
    }

    server::worker::spawn(state.clone());
    if let Some(tg) = &state.cfg.telegram {
        tokio::spawn(server::bot::run(state.clone(), tg.bot_token.clone()));
    }

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(addr = %bind_addr, "listening");
    // `into_make_service_with_connect_info` exposes the peer address to
    // handlers (used for rate limiting).
    axum::serve(listener, server::build_app(state).into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown_signal()).await?;
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
