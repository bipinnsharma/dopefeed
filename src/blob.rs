//! Vercel Blob for cached feed images, so a deployment only ships code and the
//! DB snapshot. With no `BLOB_READ_WRITE_TOKEN` every path stays local.

use reqwest::Client;
use std::sync::OnceLock;

fn token() -> Option<&'static str> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN
        .get_or_init(|| {
            let raw = std::env::var("BLOB_READ_WRITE_TOKEN").ok()?;
            let token = raw.trim().trim_matches('"').trim_matches('\'');
            (!token.is_empty()).then(|| token.to_string())
        })
        .as_deref()
}

/// `vercel_blob_rw_<store>_<secret>` carries the store id, which is also the
/// public read host.
pub fn base() -> Option<String> {
    let store = token()?
        .strip_prefix("vercel_blob_rw_")?
        .split('_')
        .next()?;
    Some(format!(
        "https://{}.public.blob.vercel-storage.com",
        store.to_ascii_lowercase()
    ))
}

/// Uploads one file under `pathname`. False when Blob is unset or refuses it.
pub async fn put(client: &Client, pathname: &str, body: Vec<u8>, content_type: &str) -> bool {
    let Some(token) = token() else {
        return false;
    };
    let result = client
        .put(format!("https://vercel.com/api/blob/?pathname={pathname}"))
        .header("authorization", format!("Bearer {token}"))
        .header("x-api-version", "12")
        .header("x-content-type", content_type)
        .header("x-add-random-suffix", "0")
        .header("x-allow-overwrite", "1")
        .body(body)
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::warn!(%pathname, status = %response.status(), "blob upload rejected");
            false
        }
        Err(error) => {
            tracing::warn!(%pathname, %error, "blob upload failed");
            false
        }
    }
}

pub fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        _ => "image/jpeg",
    }
}
