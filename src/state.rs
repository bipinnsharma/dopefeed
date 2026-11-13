use crate::{config::Config, worker};
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    /// In-memory submission rate limit: IP -> attempt timestamps.
    pub submissions: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// In-memory failed admin-key attempts: IP -> attempt timestamps.
    pub admin_attempts: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// Per-domain last fetch instant for the worker's politeness rule.
    pub domains: Arc<Mutex<HashMap<String, Instant>>>,
    /// Consecutive zero-new-post fetches per feed (adaptive interval).
    pub zero_streak: Arc<Mutex<HashMap<i64, i64>>>,
    pub fetch_slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: Config) -> anyhow::Result<Self> {
        let http = worker::fetch::http_client(config.base_url.as_str())?;
        let fetch_slots = Arc::new(Semaphore::new(config.fetch_concurrency));
        Ok(Self {
            pool,
            config: Arc::new(config),
            http,
            submissions: Arc::new(Mutex::new(HashMap::new())),
            admin_attempts: Arc::new(Mutex::new(HashMap::new())),
            domains: Arc::new(Mutex::new(HashMap::new())),
            zero_streak: Arc::new(Mutex::new(HashMap::new())),
            fetch_slots,
        })
    }
}

/// Select expression shared by every post list (timeline and feed pages).
pub const POST_COLUMNS: &str = r#"
p.id, p.feed_id, f.title AS feed_title, f.domain AS feed_domain, p.guid,
p.url, p.title, p.author, p.summary, p.content_html, p.thumb_url,
date(p.published_at) AS published_label
"#;
