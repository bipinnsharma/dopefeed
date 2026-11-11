use anyhow::Context;
use std::net::SocketAddr;
use url::Url;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub base_url: Url,
    pub database_url: String,
    pub fetch_concurrency: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".into())
            .parse()
            .context("BIND_ADDR must be a socket address")?;
        let base_url = std::env::var("BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".into())
            .parse()
            .context("BASE_URL must be an absolute URL")?;
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/dopefeed.db?mode=rwc".into());
        let fetch_concurrency = std::env::var("FETCH_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
            .max(1);
        Ok(Self {
            bind_addr,
            base_url,
            database_url,
            fetch_concurrency,
        })
    }
}
