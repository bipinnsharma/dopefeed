mod assets;
mod blob;
mod config;
mod db;
mod error;
mod models;
mod routes;
mod seed;
mod state;
#[cfg(test)]
mod tests;
mod worker;

use config::Config;
use state::AppState;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let pool = db::init(&config).await?;

    let command = std::env::args().nth(1);
    if command.as_deref() == Some("seed") {
        seed::run(pool, config).await?;
        return Ok(());
    }
    if let Some(command) = command.as_deref() {
        let state = AppState::new(pool, config)?;
        match command {
            "refresh" => worker::refresh_all(&state).await?,
            "optimize-images" => {
                assets::optimize_all(&state.pool, std::path::Path::new(assets::DIR)).await?;
            }
            "verify-assets" => assets::verify_all(&state.pool).await?,
            "sync-assets" => assets::sync_all(std::path::Path::new(assets::DIR)).await?,
            other => anyhow::bail!("unknown command {other:?}"),
        }
        return Ok(());
    }

    let state = AppState::new(pool, config)?;
    let bind_addr = state.config.bind_addr;
    let app = routes::router(state.clone());
    // Bind before the worker starts so cold instances answer immediately.
    let listener = TcpListener::bind(bind_addr).await?;

    if std::env::var("WORKER").map(|v| v != "0").unwrap_or(true) {
        worker::spawn(state);
    }
    tracing::info!(addr = %bind_addr, "dopefeed listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

async fn shutdown() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "shutdown signal failed");
    }
}
