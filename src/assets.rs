//! Local copies of remote images, so a feed's face, cover and post posters are
//! served from disk and the remote host is touched once.

use crate::worker::ssrf;
use reqwest::{
    Client,
    header::{CONTENT_TYPE, HeaderMap},
};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::Path;

/// Must stay under `static/` because that is the only directory served.
pub const DIR: &str = "static/assets";
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// Returns a public `/static/assets/...` path once the image is on disk.
/// Repeat calls stat the file instead of fetching again.
pub async fn cache_in(dir: &Path, client: &Client, url: &str, kind: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let digest = Sha256::digest(url.as_bytes());
    let stem: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    for ext in ["jpg", "png", "webp", "gif", "avif"] {
        if tokio::fs::metadata(dir.join(kind).join(format!("{stem}.{ext}")))
            .await
            .is_ok()
        {
            return Some(public_path(kind, &stem, ext));
        }
    }

    let fetched = ssrf::get(client, &parsed, &HeaderMap::new()).await.ok()?;
    if !fetched.status.is_success() || fetched.body.is_empty() || fetched.body.len() > MAX_BYTES {
        return None;
    }
    let ext = image_ext(
        fetched
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())?,
    )?;
    tokio::fs::create_dir_all(dir.join(kind)).await.ok()?;
    tokio::fs::write(dir.join(kind).join(format!("{stem}.{ext}")), &fetched.body)
        .await
        .ok()?;
    Some(public_path(kind, &stem, ext))
}

/// Only formats a browser renders inertly; SVG can carry script.
fn image_ext(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

fn public_path(kind: &str, stem: &str, ext: &str) -> String {
    format!("/static/assets/{kind}/{stem}.{ext}")
}

/// Downloads any still-remote post poster for one feed and rewrites the row.
/// A failure leaves the remote URL, so the current page keeps working.
pub async fn localize_post_thumbs(pool: &SqlitePool, client: &Client, dir: &Path, feed_id: i64) {
    // YouTube posters are derivable from the guid, so a 304 refresh can still
    // fill them in. Blog posters need the next 200 to expose their body image.
    let _ = sqlx::query(
        "UPDATE posts SET thumb_url = 'https://i.ytimg.com/vi/' || substr(guid, 10)
         || '/mqdefault.jpg'
         WHERE feed_id = ? AND thumb_url IS NULL AND guid LIKE 'yt:video:%'",
    )
    .bind(feed_id)
    .execute(pool)
    .await;
    backfill_body_thumbs(pool, feed_id).await;
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, thumb_url FROM posts WHERE feed_id = ? AND thumb_url LIKE 'http%'",
    )
    .bind(feed_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    // Time-capped so one large feed cannot starve every other feed in a tick;
    // the next refresh continues where this one stopped.
    let started = std::time::Instant::now();
    for (id, url) in rows {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            break;
        }
        if let Some(local) = cache_in(dir, client, &url, "posts").await {
            let _ = sqlx::query("UPDATE posts SET thumb_url = ? WHERE id = ?")
                .bind(local)
                .bind(id)
                .execute(pool)
                .await;
        }
    }
}

/// Rows inserted before thumbnails were stored get their first body image
/// promoted once, so older posts stop showing the placeholder.
async fn backfill_body_thumbs(pool: &SqlitePool, feed_id: i64) {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, url, content_html FROM posts
         WHERE feed_id = ? AND thumb_url IS NULL AND guid NOT LIKE 'yt:video:%'",
    )
    .bind(feed_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (id, url, html) in rows {
        if let Some(thumb) = crate::models::first_image(&html, &url) {
            let _ = sqlx::query("UPDATE posts SET thumb_url = ? WHERE id = ?")
                .bind(thumb)
                .bind(id)
                .execute(pool)
                .await;
        }
    }
}

/// Same for a feed's avatar and banner. Cheap when they are already local.
pub async fn localize_feed_images(pool: &SqlitePool, client: &Client, dir: &Path, feed_id: i64) {
    let Ok((avatar, banner)) = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT avatar_url, banner_url FROM feeds WHERE id = ?",
    )
    .bind(feed_id)
    .fetch_one(pool)
    .await
    else {
        return;
    };
    let avatar = localize_one(dir, client, avatar, "feeds").await;
    let banner = localize_one(dir, client, banner, "feeds").await;
    let _ = sqlx::query("UPDATE feeds SET avatar_url = ?, banner_url = ? WHERE id = ?")
        .bind(avatar)
        .bind(banner)
        .bind(feed_id)
        .execute(pool)
        .await;
}

/// Cache one remote URL, keeping the original if the download fails.
pub async fn cache_or_keep(client: &Client, value: Option<String>, kind: &str) -> Option<String> {
    localize_one(Path::new(DIR), client, value, kind).await
}

async fn localize_one(
    dir: &Path,
    client: &Client,
    value: Option<String>,
    kind: &str,
) -> Option<String> {
    match value {
        Some(url) if url.starts_with("http") => {
            cache_in(dir, client, &url, kind).await.or(Some(url))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::image_ext;

    #[test]
    fn only_inert_image_types_are_cached() {
        assert_eq!(image_ext("image/jpeg; charset=binary"), Some("jpg"));
        assert_eq!(image_ext("IMAGE/PNG"), Some("png"));
        assert_eq!(image_ext("image/svg+xml"), None);
        assert_eq!(image_ext("text/html"), None);
    }
}
