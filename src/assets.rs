//! Local copies of remote images, so a feed's face, cover and post posters are
//! served from disk and the remote host is touched once.

use crate::worker::ssrf;
use image::{DynamicImage, ImageFormat, codecs::jpeg::JpegEncoder};
use reqwest::{
    Client,
    header::{CONTENT_TYPE, HeaderMap},
};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::io::Cursor;
use std::path::Path;
use tokio::fs;

/// Must stay under `static/` because that is the only directory served.
pub const DIR: &str = "static/assets";
const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Files already at their target width and this small are left alone.
const SMALL_BYTES: u64 = 100 * 1024;

/// Returns a public `/static/assets/...` path once the image is on disk.
/// Repeat calls stat the file instead of fetching again.
pub async fn cache_in(dir: &Path, client: &Client, url: &str, kind: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let digest = Sha256::digest(url.as_bytes());
    let stem: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    for ext in ["jpg", "png", "webp", "gif", "avif", "ico"] {
        if fs::metadata(dir.join(kind).join(format!("{stem}.{ext}")))
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
    fs::create_dir_all(dir.join(kind)).await.ok()?;
    let Some(width) = target_width(kind) else {
        fs::write(dir.join(kind).join(format!("{stem}.{ext}")), &fetched.body)
            .await
            .ok()?;
        return Some(public_path(kind, &stem, ext));
    };
    // Undecodable bytes (html errors, odd formats) keep the original file.
    let Ok(image) = image::load_from_memory(&fetched.body) else {
        fs::write(dir.join(kind).join(format!("{stem}.{ext}")), &fetched.body)
            .await
            .ok()?;
        return Some(public_path(kind, &stem, ext));
    };
    let (data, out_ext) = encode(&image, width)?;
    fs::write(dir.join(kind).join(format!("{stem}.{out_ext}")), &data)
        .await
        .ok()?;
    if kind == "posts" {
        let (cover, cover_ext) = encode(&image, 1200)?;
        fs::write(
            dir.join(kind).join(format!("{stem}-1200.{cover_ext}")),
            &cover,
        )
        .await
        .ok()?;
    }
    Some(public_path(kind, &stem, out_ext))
}

/// List rows render 96px, covers 1200px. `None` means "store as downloaded".
fn target_width(kind: &str) -> Option<u32> {
    match kind {
        "posts" => Some(320),
        "feeds" => Some(96),
        "banners" => Some(1200),
        _ => None,
    }
}

/// Downscale to `width` and re-encode: PNG keeps transparency, everything else
/// becomes a small JPEG. Animated inputs lose all but their first frame.
fn encode(image: &DynamicImage, width: u32) -> Option<(Vec<u8>, &'static str)> {
    let scaled = image.thumbnail(width, u32::MAX);
    let transparent =
        scaled.color().has_alpha() && scaled.to_rgba8().pixels().any(|pixel| pixel.0[3] < 255);
    if transparent {
        let mut out = Cursor::new(Vec::new());
        scaled.write_to(&mut out, ImageFormat::Png).ok()?;
        Some((out.into_inner(), "png"))
    } else {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 82)
            .encode_image(&DynamicImage::ImageRgb8(scaled.to_rgb8()))
            .ok()?;
        Some((out, "jpg"))
    }
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
        "image/x-icon" | "image/vnd.microsoft.icon" => Some("ico"),
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
        // An empty string records "no body image" so the IS NULL query above
        // does not re-parse this article on every refresh.
        let thumb = crate::models::first_image(&html, &url).unwrap_or_default();
        let _ = sqlx::query("UPDATE posts SET thumb_url = ? WHERE id = ?")
            .bind(thumb)
            .bind(id)
            .execute(pool)
            .await;
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
    let banner = localize_one(dir, client, banner, "banners").await;
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

/// One-off pass over images cached at full size. Returns how many post and
/// feed images were rewritten.
#[allow(dead_code)] // run once by hand; no request path calls it
pub async fn optimize_all(pool: &SqlitePool, dir: &Path) -> anyhow::Result<(usize, usize)> {
    let mut posts = 0;
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT thumb_url FROM posts WHERE thumb_url LIKE '/static/assets/posts/%'",
    )
    .fetch_all(pool)
    .await?;
    for path in paths {
        if let Some(new) = optimize_file(dir, &path, "posts", 320).await {
            if new != path {
                sqlx::query("UPDATE posts SET thumb_url = ? WHERE thumb_url = ?")
                    .bind(&new)
                    .bind(&path)
                    .execute(pool)
                    .await?;
            }
            posts += 1;
        }
    }

    let mut feeds = 0;
    let avatars: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT avatar_url FROM feeds WHERE avatar_url LIKE '/static/assets/feeds/%'",
    )
    .fetch_all(pool)
    .await?;
    for path in avatars {
        if let Some(new) = optimize_file(dir, &path, "feeds", 96).await {
            if new != path {
                sqlx::query("UPDATE feeds SET avatar_url = ? WHERE avatar_url = ?")
                    .bind(&new)
                    .bind(&path)
                    .execute(pool)
                    .await?;
            }
            feeds += 1;
        }
    }

    // Banners that used to live under feeds/ move out, so avatar sizing can
    // never clamp a cover.
    let banners: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT banner_url FROM feeds
         WHERE banner_url LIKE '/static/assets/feeds/%'
            OR banner_url LIKE '/static/assets/banners/%'",
    )
    .fetch_all(pool)
    .await?;
    for path in banners {
        if let Some(new) = optimize_file(dir, &path, "banners", 1200).await {
            if new != path {
                sqlx::query("UPDATE feeds SET banner_url = ? WHERE banner_url = ?")
                    .bind(&new)
                    .bind(&path)
                    .execute(pool)
                    .await?;
            }
            feeds += 1;
        }
    }
    tracing::info!(posts, feeds, "optimized local assets");
    Ok((posts, feeds))
}

/// Re-encode one cached file at `width`, writing it under `kind`'s folder.
/// `None` means skip: undecodable, already small, or IO failed.
async fn optimize_file(dir: &Path, public: &str, kind: &str, width: u32) -> Option<String> {
    let name = public.rsplit('/').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    // Icon and AVIF decoders are unreliable here; a broken rewrite is worse
    // than a large file.
    if matches!(ext.to_ascii_lowercase().as_str(), "ico" | "avif") {
        return None;
    }
    let old = dir.join(public.strip_prefix("/static/assets/")?);
    let bytes = fs::read(&old).await.ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    if decoded.width() <= width && bytes.len() as u64 <= SMALL_BYTES {
        return None;
    }
    let (data, out_ext) = encode(&decoded, width)?;
    let cover = if kind == "posts" {
        Some(encode(&decoded, 1200)?)
    } else {
        None
    };
    let out_dir = dir.join(kind);
    fs::create_dir_all(&out_dir).await.ok()?;
    let out = out_dir.join(format!("{stem}.{out_ext}"));
    fs::write(&out, &data).await.ok()?;
    if let Some((cover, cover_ext)) = cover {
        fs::write(out_dir.join(format!("{stem}-1200.{cover_ext}")), &cover)
            .await
            .ok()?;
    }
    if out != old {
        fs::remove_file(&old).await.ok();
    }
    Some(public_path(kind, stem, out_ext))
}

#[cfg(test)]
mod tests {
    use super::{encode, image_ext};
    use image::{DynamicImage, Rgba, RgbaImage};

    #[test]
    fn only_inert_image_types_are_cached() {
        assert_eq!(image_ext("image/jpeg; charset=binary"), Some("jpg"));
        assert_eq!(image_ext("IMAGE/PNG"), Some("png"));
        assert_eq!(image_ext("image/x-icon"), Some("ico"));
        assert_eq!(image_ext("image/svg+xml"), None);
        assert_eq!(image_ext("text/html"), None);
    }

    #[test]
    fn encode_clamps_width_and_picks_a_format() {
        let opaque =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([1, 2, 3, 255])));
        let (bytes, ext) = encode(&opaque, 50).unwrap();
        assert_eq!(ext, "jpg");
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (50, 25));

        let transparent =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([1, 2, 3, 0])));
        let (bytes, ext) = encode(&transparent, 50).unwrap();
        assert_eq!(ext, "png");
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 50);
    }
}
