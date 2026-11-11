use crate::{config::Config, routes::submit, state::AppState, worker::discover};
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use url::Url;

const SEED: &[(&str, &str)] = &[
    ("https://daringfireball.net/feeds/main", "tech"),
    ("https://simonwillison.net/atom/everything/", "coding"),
    ("https://www.theverge.com/rss/index.xml", "tech"),
    ("https://css-tricks.com/feed/", "coding"),
    ("https://github.blog/feed/", "product"),
    ("http://www.aaronsw.com/2002/feeds/pgessays.rss", "ceo"),
    ("http://antirez.com/rss", "coding"),
    ("https://mitchellh.com/feed.xml", "coding"),
    ("https://danluu.com/atom.xml", "coding"),
    ("https://www.ruanyifeng.com/blog/atom.xml", "coding"),
    ("https://blog.samaltman.com/posts.atom", "ceo"),
    ("https://karpathy.bearblog.dev/feed/", "ai"),
    ("https://www.joshwcomeau.com/rss.xml", "coding"),
    ("https://maggieappleton.com/rss.xml", "ai"),
    ("https://sive.rs/en.atom", "writing"),
    ("https://zed.dev/blog.rss", "product"),
];

pub async fn run(pool: SqlitePool, config: Config) -> anyhow::Result<()> {
    let state = AppState::new(pool, config)?;
    let mut tasks = JoinSet::new();

    for (raw_url, category) in SEED {
        let state = state.clone();
        tasks.spawn(async move {
            let _permit = state.fetch_slots.clone().acquire_owned().await;
            let Ok(url) = Url::parse(raw_url) else {
                tracing::warn!(url = raw_url, "invalid seed url");
                return;
            };
            match discover::resolve(&state.http, &url).await {
                Ok(resolved) => {
                    match submit::insert_resolved(&state, resolved, category, None).await {
                        Ok(Some(id)) => tracing::info!(%url, id, "seeded"),
                        Ok(None) => tracing::info!(%url, "already present"),
                        Err(error) => tracing::warn!(%url, %error, "seed insert failed"),
                    }
                }
                Err(error) => {
                    tracing::warn!(%url, %error, "seed fetch failed, adding placeholder");
                    if let Err(error) = insert_placeholder(&state, raw_url, category).await {
                        tracing::warn!(%url, %error, "placeholder insert failed");
                    }
                }
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            tracing::warn!(%error, "seed task failed");
        }
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await?;
    tracing::info!(feeds = total, "seed complete");
    Ok(())
}

/// Offline fallback so a fresh clone has a populated directory without network.
async fn insert_placeholder(state: &AppState, raw_url: &str, category: &str) -> anyhow::Result<()> {
    let url = Url::parse(raw_url)?;
    let domain = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string();
    let title = domain.clone();
    let site_url = url.origin().ascii_serialization();
    sqlx::query(
        "INSERT INTO feeds (url, site_url, domain, title, category, status, next_fetch_at)
         VALUES (?, ?, ?, ?, ?, 'active', datetime('now', '+60 seconds'))
         ON CONFLICT(url) DO NOTHING",
    )
    .bind(url.to_string())
    .bind(site_url)
    .bind(domain)
    .bind(title)
    .bind(category)
    .execute(&state.pool)
    .await?;
    Ok(())
}
