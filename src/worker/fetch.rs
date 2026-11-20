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
use std::{collections::HashMap, sync::Arc, time::Duration};
use url::Url;

const MAX_ENTRIES: usize = 1000;
const BODY_URL_VERSION: &str = "resolved-body-urls-v1";
const BODY_REPAIR_BATCH: i64 = 500;

/// Feeds are allowed to link with site-relative paths (pj-onori.blog does).
fn absolute(href: &str, base: &str) -> String {
    match Url::parse(href) {
        Ok(url) => url.to_string(),
        Err(_) => Url::parse(base)
            .and_then(|base| base.join(href))
            .map(|url| url.to_string())
            .unwrap_or_else(|_| href.to_string()),
    }
}

fn clean_body(raw: &str, article_url: &str) -> String {
    match Url::parse(article_url) {
        Ok(base) => ammonia::Builder::new()
            .url_relative(ammonia::UrlRelative::RewriteWithBase(base))
            .clean(raw)
            .to_string(),
        Err(_) => ammonia::clean(raw),
    }
}

/// Rewrites already-stored bodies, including posts no longer returned by feeds.
pub async fn repair_body_urls(pool: &SqlitePool) -> anyhow::Result<usize> {
    let mut after_id = 0;
    let mut changed = 0;
    loop {
        let rows: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT id, url, content_html FROM posts
             WHERE id > ? AND content_html <> '' ORDER BY id LIMIT ?",
        )
        .bind(after_id)
        .bind(BODY_REPAIR_BATCH)
        .fetch_all(pool)
        .await?;
        let Some((last_id, _, _)) = rows.last() else {
            break;
        };
        after_id = *last_id;
        for (id, url, old_html) in rows {
            let new_html = clean_body(&old_html, &url);
            if new_html != old_html {
                sqlx::query("UPDATE posts SET content_html = ? WHERE id = ?")
                    .bind(new_html)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

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
    pub avatar_url: Option<String>,
    pub favicon_fetched_at: Option<String>,
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
    if feed.category == "youtube" {
        backfill_channel_images(state, feed).await?;
    } else {
        backfill_site_images(state, feed).await?;
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
            if !fallback_to_channel_page(state, feed).await? {
                record_failure(state, feed, &error).await?;
            }
            return Ok(());
        }
    };

    if fetched.status == reqwest::StatusCode::NOT_MODIFIED {
        succeed(state, feed, 0, &fetched.headers).await?;
        return Ok(());
    }
    if !fetched.status.is_success() {
        if !fallback_to_channel_page(state, feed).await? {
            record_failure(state, feed, &format!("HTTP {}", fetched.status.as_u16())).await?;
        }
        return Ok(());
    }

    let Some(parsed) = discover::parse_feed(&fetched.body) else {
        if fallback_to_channel_page(state, feed).await? {
            return Ok(());
        }
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
/// means channels without artwork are not refetched on every refresh. Also
/// rewrites an already-fetched remote avatar/banner to a local copy.
async fn backfill_channel_images(state: &AppState, feed: &DueFeed) -> anyhow::Result<()> {
    if feed.images_fetched_at.is_none()
        && let Some(channel_id) = discover::channel_id_from_feed_url(&feed.url)
        && let Some(images) = discover::fetch_channel_images(&state.http, &channel_id).await
    {
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
    }
    crate::assets::localize_feed_images(
        &state.pool,
        &state.http,
        std::path::Path::new(crate::assets::DIR),
        feed.id,
    )
    .await;
    Ok(())
}

/// YouTube's RSS endpoint is a separate service from the channel page. When it
/// is down, ingest the page's video list so posts still arrive.
async fn fallback_to_channel_page(state: &AppState, feed: &DueFeed) -> anyhow::Result<bool> {
    if feed.category != "youtube" {
        return Ok(false);
    }
    let Some(channel_id) = discover::channel_id_from_feed_url(&feed.url) else {
        return Ok(false);
    };
    let Some(parsed) = discover::fetch_channel_videos(&state.http, &channel_id).await else {
        return Ok(false);
    };
    let new = ingest(&state.pool, feed.id, &parsed, &feed.site_url).await?;
    // A synthesized feed has no HTTP validators to cache.
    succeed(state, feed, new, &HeaderMap::new()).await?;
    Ok(true)
}

/// Non-YouTube feeds use the site's og:image as their cover and its icon as
/// their avatar. One page fetch fills whichever is missing; each marker column
/// stops a site without that image from being refetched on every tick.
async fn backfill_site_images(state: &AppState, feed: &DueFeed) -> anyhow::Result<()> {
    let need_cover = feed.images_fetched_at.is_none();
    let need_icon = feed.avatar_url.as_deref().unwrap_or_default().is_empty()
        && feed.favicon_fetched_at.is_none();
    if !need_cover && !need_icon {
        return Ok(());
    }
    let Ok(url) = Url::parse(&feed.site_url) else {
        return Ok(());
    };
    // A failed fetch must not set the markers, so both are retried later.
    let Ok(fetched) = ssrf::get(&state.http, &url, &HeaderMap::new()).await else {
        return Ok(());
    };
    if !fetched.status.is_success() {
        return Ok(());
    }
    let html = std::str::from_utf8(&fetched.body).ok();

    let banner = if need_cover {
        let image = html.and_then(|html| discover::extract_og_image(html, &fetched.final_url));
        // A remote URL that could not be cached is likely dead (an expired CDN
        // preview); no cover beats a broken one.
        crate::assets::cache_or_keep(&state.http, image, "banners")
            .await
            .filter(|url| !url.starts_with("http"))
    } else {
        None
    };

    let avatar = if need_icon {
        let mut local = None;
        for candidate in html
            .map(|html| discover::extract_icons(html, &fetched.final_url))
            .unwrap_or_default()
        {
            if let Some(path) =
                crate::assets::cache_or_keep(&state.http, Some(candidate), "feeds").await
                && !path.starts_with("http")
            {
                local = Some(path);
                break;
            }
        }
        local
    } else {
        None
    };

    let now = sqlite_time(Utc::now());
    sqlx::query(
        "UPDATE feeds SET banner_url = COALESCE(banner_url, ?),
         images_fetched_at = COALESCE(images_fetched_at, ?),
         avatar_url = COALESCE(avatar_url, ?),
         favicon_fetched_at = COALESCE(favicon_fetched_at, ?) WHERE id = ?",
    )
    .bind(banner)
    .bind(need_cover.then(|| now.clone()))
    .bind(avatar)
    .bind(need_icon.then_some(now))
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
    // New posts were inserted after the start-of-refresh pass.
    crate::assets::localize_post_thumbs(
        &state.pool,
        &state.http,
        std::path::Path::new(crate::assets::DIR),
        feed.id,
    )
    .await;
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
    let rows: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT guid, content_hash, url FROM posts WHERE feed_id = ?")
            .bind(feed_id)
            .fetch_all(pool)
            .await?;
    let mut existing: HashMap<String, Option<String>> = HashMap::new();
    let mut by_url: HashMap<String, String> = HashMap::new();
    for (guid, hash, url) in rows {
        by_url.insert(url, guid.clone());
        existing.insert(guid, hash);
    }

    let mut new_posts = 0;
    for entry in feed.entries.iter().take(MAX_ENTRIES) {
        let link = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref().is_none_or(|rel| rel == "alternate"))
            .or_else(|| entry.links.first())
            .map(|l| absolute(&l.href, fallback_url));
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
        let published = entry.published.or(entry.updated).map(sqlite_time);

        let raw = entry
            .content
            .as_ref()
            .and_then(|c| c.body.clone())
            .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()))
            .unwrap_or_default();
        let entry_hash = content_hash(&url, &display_title, &raw, published.as_deref());
        // A link-only stub stores guid = url; the RSS entry carries its own
        // guid. Match by URL so the refresh fills the stub instead of adding a
        // second row for the same article.
        let stored_guid = if existing.contains_key(&guid) {
            guid.clone()
        } else {
            by_url.get(&url).cloned().unwrap_or_else(|| guid.clone())
        };
        if existing
            .get(&stored_guid)
            .is_some_and(|stored| stored.as_deref() == Some(entry_hash.as_str()))
        {
            continue;
        }
        // Decoded lazily: body images sit below the fold and the reader
        // should not fetch them all at once.
        let content_html =
            clean_body(&raw, &url).replace("<img ", "<img loading=\"lazy\" decoding=\"async\" ");
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
        // Stored remote first, then rewritten by assets::localize_post_thumbs.
        let thumb = crate::models::youtube_thumb(&guid)
            .or_else(|| crate::models::first_image(&content_html, &url));

        if existing.contains_key(&stored_guid) {
            // Only move a date backwards: a feed with no real date must not
            // keep pushing old posts to the top of Recent.
            sqlx::query(
                "UPDATE posts SET title = ?, content_html = ?, summary = ?, author = ?,
                 thumb_url = COALESCE(NULLIF(thumb_url, ''), ?), content_hash = ?,
                 published_at = CASE WHEN ? < published_at THEN ? ELSE published_at END
                 WHERE feed_id = ? AND guid = ?",
            )
            .bind(&display_title)
            .bind(&content_html)
            .bind(&summary)
            .bind(&author)
            .bind(&thumb)
            .bind(&entry_hash)
            .bind(&published)
            .bind(&published)
            .bind(feed_id)
            .bind(&stored_guid)
            .execute(pool)
            .await?;
            existing.insert(stored_guid.clone(), Some(entry_hash.clone()));
            by_url.insert(url.clone(), stored_guid);
        } else {
            sqlx::query(
                "INSERT INTO posts (feed_id, guid, url, title, author, content_html, summary,
                 thumb_url, published_at, content_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(feed_id)
            .bind(&guid)
            .bind(&url)
            .bind(&display_title)
            .bind(&author)
            .bind(&content_html)
            .bind(&summary)
            .bind(&thumb)
            .bind(published.unwrap_or_else(|| sqlite_time(Utc::now())))
            .bind(&entry_hash)
            .execute(pool)
            .await?;
            existing.insert(guid.clone(), Some(entry_hash));
            by_url.insert(url, guid);
            new_posts += 1;
        }
    }
    Ok(new_posts)
}

fn hash(title: &str, link: &str) -> String {
    let digest = Sha256::digest(format!("{title}{link}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash of the entry inputs that decide the stored row, so an unchanged entry
/// can skip sanitizing and extracting its body.
fn content_hash(url: &str, title: &str, raw: &str, published: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    // Version the inputs so content-hash skips cannot leave old bodies unresolved.
    for part in [
        BODY_URL_VERSION,
        url,
        title,
        raw,
        published.unwrap_or_default(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
