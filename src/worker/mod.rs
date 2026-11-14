pub mod discover;
pub mod fetch;
pub mod ssrf;

use crate::state::AppState;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

const TICK: Duration = Duration::from_secs(60);
const DOMAIN_GAP: Duration = Duration::from_secs(2);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            if let Err(error) = run_tick(&state).await {
                tracing::warn!(%error, "worker tick failed");
            }
        }
    });
}

async fn run_tick(state: &AppState) -> anyhow::Result<()> {
    let due = sqlx::query_as::<_, fetch::DueFeed>(
        "SELECT id, url, site_url, etag, last_modified, fetch_interval_secs, error_count, domain,
         category, images_fetched_at, avatar_url, favicon_fetched_at
         FROM feeds WHERE status = 'active' AND next_fetch_at <= datetime('now')
         ORDER BY next_fetch_at ASC LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut tasks = JoinSet::new();
    for feed in due {
        if !claim_domain(state, &feed.domain) {
            continue;
        }
        let state = state.clone();
        tasks.spawn(async move {
            let _permit = state.fetch_slots.clone().acquire_owned().await;
            if let Err(error) = fetch::refresh(&state, &feed).await {
                tracing::warn!(feed_id = feed.id, %error, "feed refresh failed");
            }
        });
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

/// One fetch per domain at a time, with at least 2s between attempts.
fn claim_domain(state: &AppState, domain: &str) -> bool {
    let Ok(mut domains) = state.domains.lock() else {
        return false;
    };
    let now = Instant::now();
    if let Some(last) = domains.get(domain)
        && now.duration_since(*last) < DOMAIN_GAP
    {
        return false;
    }
    domains.insert(domain.to_string(), now);
    true
}
