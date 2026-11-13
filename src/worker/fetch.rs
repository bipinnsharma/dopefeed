use super::{discover, ssrf};
use crate::state::AppState;
use anyhow::anyhow;
use chrono::{DateTime, Utc};
use reqwest::{
    Client,
    header::{ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
};
use scraper::Html;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{collections::HashSet, sync::Arc, time::Duration};
use url::Url;

const MAX_ENTRIES: usize = 500;

#[derive(Debug, sqlx::FromRow)]
pub struct DueFeed {
    pub id: i64,
    pub url: String,
    pub site_url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub fetch_interval_secs: i64,
    pub error_count: i64,
    pub domain: String,
    pub category: String,
    pub images_fetched_at: Option<String>,
}

pub fn http_client(base_url: &str) -> anyhow::Result<Client> {
    let resolver: Arc<dyn reqwest::dns::Resolve> = Arc::new(ssrf::SafeResolver);
    Ok(Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent(format!("dopefeed/1.0 (+{base_url})"))
        .redirect(reqwest::redirect::Policy::none())
        .gzip(true)
        .dns_resolver(resolver)
        .build()?)
}

pub fn sqlite_time(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Fetches one feed and persists the result. Fetch/parse failures update backoff
/// instead of returning an error; only database failures bubble up.
pub async fn refresh(state: &AppState, feed: &DueFeed) -> anyhow::Result<()> {
    if feed.category == "youtube" && feed.images_fetched_at.is_none() {
        backfill_channel_images(state, feed).await?;
    }

    let url = Url::parse(&feed.url).map_err(|e| anyhow!("bad feed url: {e}"))?;
    let mut headers = HeaderMap::new();
    if let Some(etag) = &feed.etag
        && let Ok(value) = HeaderValue::from_str(etag)
    {
        headers.insert(IF_NONE_MATCH, value);
    }
    if let Some(modified) = &feed.last_modified
        && let Ok(value) = HeaderValue::from_str(modified)
    {
        headers.insert(IF_MODIFIED_SINCE, value);
    }

    let fetched = match ssrf::get(&state.http, &url, &headers).await {
        Ok(fetched) => fetched,
        Err(error) => {
            record_failure(state, feed, &error).await?;
            return Ok(());
        }
    };

    if fetched.status == reqwest::StatusCode::NOT_MODIFIED {
        succeed(state, feed, 0, &fetched.headers).await?;
        return Ok(());
    }
    if !fetched.status.is_success() {
        record_failure(state, feed, &format!("HTTP {}", fetched.status.as_u16())).await?;
        return Ok(());
    }

    let Some(parsed) = discover::parse_feed(&fetched.body) else {
        return retry_discovery(state, feed).await;
    };

    if let Some(title) = parsed
        .title
        .as_ref()
        .map(|t| t.content.trim())
        .filter(|t| !t.is_empty())
    {
        sqlx::query(
            "UPDATE feeds SET title = ? WHERE id = ?
             AND (SELECT COUNT(*) FROM posts WHERE feed_id = ?) = 0",
        )
        .bind(title)
        .bind(feed.id)
        .bind(feed.id)
        .execute(&state.pool)
        .await?;
    }

    let new = ingest(&state.pool, feed.id, &parsed, &feed.site_url).await?;
    succeed(state, feed, new, &fetched.headers).await?;
    Ok(())
}

/// Fetches the channel page once to learn the avatar/banner. The marker column
/// means channels without artwork are not refetched on every refresh.
async fn backfill_channel_images(state: &AppState, feed: &DueFeed) -> anyhow::Result<()> {
    let Some(channel_id) = discover::channel_id_from_feed_url(&feed.url) else {
        return Ok(());
    };
    let Some(images) = discover::fetch_channel_images(&state.http, &channel_id).await else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE feeds SET avatar_url = COALESCE(?, avatar_url),
         banner_url = COALESCE(?, banner_url), images_fetched_at = ? WHERE id = ?",
    )
    .bind(images.avatar)
    .bind(images.banner)
    .bind(sqlite_time(Utc::now()))
    .bind(feed.id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// A row may be a placeholder (site page instead of a feed) or an HTML/sitemap
/// synthetic feed. Retry discovery on parse failure, even when posts already
/// exist, then switch the row to whatever discovery resolved.
async fn retry_discovery(state: &AppState, feed: &DueFeed) -> anyhow::Result<()> {
    let url = Url::parse(&feed.url).map_err(|e| anyhow!("bad feed url: {e}"))?;
    match discover::resolve(&state.http, &url).await {
        Ok(resolved) => {
            sqlx::query(
                "UPDATE feeds SET url = ?, site_url = ?, description = COALESCE(?, description),
                 etag = NULL, last_modified = NULL
                 WHERE id = ?",
            )
            .bind(&resolved.feed_url)
            .bind(&resolved.site_url)
            .bind(&resolved.description)
            .bind(feed.id)
            .execute(&state.pool)
            .await?;
            if let Some(title) = resolved.title.as_deref() {
                sqlx::query(
                    "UPDATE feeds SET title = ? WHERE id = ?
                     AND (SELECT COUNT(*) FROM posts WHERE feed_id = ?) = 0",
                )
                .bind(title)
                .bind(feed.id)
                .bind(feed.id)
                .execute(&state.pool)
                .await?;
            }
            let new = ingest(&state.pool, feed.id, &resolved.feed, &resolved.site_url).await?;
            // Never cache ETag/Last-Modified here: for a synthetic feed they
            // describe the page and would turn the next refresh into a 304.
            succeed(state, feed, new, &HeaderMap::new()).await
        }
        Err(error) => record_failure(state, feed, &error).await,
    }
}

async fn succeed(
    state: &AppState,
    feed: &DueFeed,
    new_posts: usize,
    headers: &HeaderMap,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let mut interval = feed.fetch_interval_secs;
    let mut streak = *state
        .zero_streak
        .lock()
        .map_err(|_| anyhow!("zero streak lock poisoned"))?
        .get(&feed.id)
        .unwrap_or(&0);

    if new_posts >= 3 {
        interval = interval.max(900) / 2;
        interval = interval.max(900);
        streak = 0;
    } else if new_posts == 0 {
        streak += 1;
        if streak >= 10 {
            interval = (interval * 2).min(86400);
            streak = 0;
        }
    } else {
        streak = 0;
    }
    state
        .zero_streak
        .lock()
        .map_err(|_| anyhow!("zero streak lock poisoned"))?
        .insert(feed.id, streak);

    let etag = ssrf::header_value(headers, ETAG);
    let last_modified = ssrf::header_value(headers, LAST_MODIFIED);

    sqlx::query(
        "UPDATE feeds SET etag = ?, last_modified = ?, last_fetched_at = ?,
         next_fetch_at = ?, fetch_interval_secs = ?, error_count = 0, last_error = NULL
         WHERE id = ?",
    )
    .bind(etag)
    .bind(last_modified)
    .bind(sqlite_time(now))
    .bind(sqlite_time(now + chrono::Duration::seconds(interval)))
    .bind(interval)
    .bind(feed.id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn record_failure(state: &AppState, feed: &DueFeed, error: &str) -> anyhow::Result<()> {
    let now = Utc::now();
    let error_count = feed.error_count + 1;
    let exponent = error_count.clamp(1, 6) as u32;
    let backoff = feed
        .fetch_interval_secs
        .saturating_mul(1i64 << exponent)
        .min(86400);
    let last_error: String = error.chars().take(500).collect();

    sqlx::query(
        "UPDATE feeds SET error_count = ?, last_error = ?, last_fetched_at = ?, next_fetch_at = ?
         WHERE id = ?",
    )
    .bind(error_count)
    .bind(last_error)
    .bind(sqlite_time(now))
    .bind(sqlite_time(now + chrono::Duration::seconds(backoff)))
    .bind(feed.id)
    .execute(&state.pool)
    .await?;
    tracing::debug!(feed_id = feed.id, error, "feed fetch failed");
    Ok(())
}

/// Inserts/updates up to 500 entries. Returns how many posts were new.
pub async fn ingest(
    pool: &SqlitePool,
    feed_id: i64,
    feed: &feed_rs::model::Feed,
    fallback_url: &str,
) -> anyhow::Result<usize> {
    let existing: HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT guid FROM posts WHERE feed_id = ?")
            .bind(feed_id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    let mut new_posts = 0;
    for entry in feed.entries.iter().take(MAX_ENTRIES) {
        let link = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref().is_none_or(|rel| rel == "alternate"))
            .or_else(|| entry.links.first())
            .map(|l| l.href.clone());
        let title = entry
            .title
            .as_ref()
            .map(|t| t.content.trim().to_string())
            .unwrap_or_default();

        if title.is_empty() && link.is_none() {
            continue;
        }
        let guid = if entry.id.trim().is_empty() {
            link.clone().unwrap_or_else(|| hash(&title, fallback_url))
        } else {
            entry.id.clone()
        };
        let url = link.unwrap_or_else(|| fallback_url.to_string());
        let display_title = if title.is_empty() {
            "(untitled)".to_string()
        } else {
            title
        };
        let published = entry
            .published
            .or(entry.updated)
            .map(sqlite_time)
            .unwrap_or_else(|| sqlite_time(Utc::now()));

        let raw = entry
            .content
            .as_ref()
            .and_then(|c| c.body.clone())
            .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()))
            .unwrap_or_default();
        let content_html = ammonia::clean(&raw);
        let summary_source = entry
            .summary
            .as_ref()
            .map(|s| s.content.clone())
            .unwrap_or_else(|| content_html.clone());
        let summary: String = plain_text(&summary_source).chars().take(300).collect();
        let author = entry.authors.first().and_then(|p| {
            p.name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| p.email.clone())
        });

        if existing.contains(&guid) {
            sqlx::query(
                "UPDATE posts SET title = ?, content_html = ?, summary = ?, author = ?
                 WHERE feed_id = ? AND guid = ?",
            )
            .bind(&display_title)
            .bind(&content_html)
            .bind(&summary)
            .bind(&author)
            .bind(feed_id)
            .bind(&guid)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO posts (feed_id, guid, url, title, author, content_html, summary,
                 published_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(feed_id)
            .bind(&guid)
            .bind(&url)
            .bind(&display_title)
            .bind(&author)
            .bind(&content_html)
            .bind(&summary)
            .bind(&published)
            .execute(pool)
            .await?;
            new_posts += 1;
        }
    }
    Ok(new_posts)
}

fn hash(title: &str, link: &str) -> String {
    let digest = Sha256::digest(format!("{title}{link}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn plain_text(html: &str) -> String {
    let document = Html::parse_fragment(html);
    document
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
