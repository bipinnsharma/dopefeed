use anyhow::Context;
use std::net::SocketAddr;
use url::Url;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub base_url: Url,
    pub database_url: String,
    pub fetch_concurrency: usize,
    /// Six alphanumeric characters (letters + digits) guarding `/admin`.
    /// `None` disables the admin routes.
    pub admin_key: Option<String>,
}

pub fn valid_admin_key(key: &str) -> bool {
    key.len() == 6
        && key.chars().all(|c| c.is_ascii_alphanumeric())
        && key.chars().any(|c| c.is_ascii_digit())
        && key.chars().any(|c| c.is_ascii_alphabetic())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = match std::env::var("BIND_ADDR") {
            Ok(addr) => addr,
            // Vercel assigns the port; bind publicly inside the container.
            Err(_) => match std::env::var("PORT") {
                Ok(port) => format!("0.0.0.0:{port}"),
                Err(_) => "127.0.0.1:3000".into(),
            },
        }
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
        let admin_key = std::env::var("ADMIN_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        if let Some(key) = &admin_key {
            anyhow::ensure!(
                valid_admin_key(key),
                "ADMIN_KEY must be 6 letters/numbers with at least one of each"
            );
        }
        Ok(Self {
            bind_addr,
            base_url,
            database_url,
            fetch_concurrency,
            admin_key,
        })
    }
}
