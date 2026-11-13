use super::ssrf;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use reqwest::{
    Client,
    header::{COOKIE, ETAG, HeaderMap, HeaderValue, LAST_MODIFIED, USER_AGENT},
};
use scraper::{Html, Selector};
use url::Url;

const YOUTUBE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
const YOUTUBE_DESKTOP_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";
const YOUTUBE_COOKIE: &str = "CONSENT=YES+cb.20210328-17-p0.en+FX+417; SOCS=CAI";

pub struct Resolved {
    pub feed_url: String,
    pub site_url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    /// True when the channel page was fetched, even if it had no artwork.
    pub images_fetched: bool,
    pub feed: feed_rs::model::Feed,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// True when the feed was synthesized from HTML/sitemap rather than parsed
    /// as RSS/Atom. Its ETag is for the page, so it must not be stored.
    pub synthetic: bool,
}

/// Turns a feed URL, a site URL, or a YouTube channel URL into a parsed feed.
pub async fn resolve(client: &Client, input: &Url) -> Result<Resolved, String> {
    if is_youtube(input) {
        return resolve_youtube(client, input).await;
    }

    let fetched = ssrf::get(client, input, &HeaderMap::new()).await?;
    if !fetched.status.is_success() {
        return Err(format!(
            "That URL returned HTTP {}.",
            fetched.status.as_u16()
        ));
    }

    if let Some(feed) = parse_feed(&fetched.body) {
        return Ok(build(
            fetched.final_url,
            feed,
            fetched.headers,
            input,
            false,
        ));
    }

    if let Ok(html) = std::str::from_utf8(&fetched.body) {
        for candidate in alternates(html, &fetched.final_url) {
            let Ok(next) = ssrf::get(client, &candidate, &HeaderMap::new()).await else {
                continue;
            };
            if let Some(feed) = parse_feed(&next.body) {
                return Ok(build(next.final_url, feed, next.headers, input, false));
            }
        }
        if let Some(feed) = html_feed(html, &fetched.final_url) {
            return Ok(build(
                fetched.final_url.clone(),
                feed,
                fetched.headers,
                input,
                true,
            ));
        }
    }
    if let Some(feed) = sitemap_feed(client, &fetched.final_url).await {
        return Ok(build(
            fetched.final_url.clone(),
            feed,
            fetched.headers,
            input,
            true,
        ));
    }

    Err("Could not find a feed at that URL. Try the feed link or a YouTube channel.".into())
}

/// Many personal sites list their posts in HTML but publish no RSS. Build a
/// synthetic Atom feed from the largest group of same-site article links.
pub(crate) fn html_feed(html: &str, base: &Url) -> Option<feed_rs::model::Feed> {
    use std::collections::HashMap;

    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").ok()?;
    let mut by_prefix: HashMap<String, HashMap<String, String>> = HashMap::new();
    for element in document.select(&selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        if href.starts_with('#')
            || href.starts_with('?')
            || href.starts_with("mailto:")
            || href.starts_with("javascript:")
        {
            continue;
        }
        let Ok(url) = base.join(href) else {
            continue;
        };
        if url.host_str() != base.host_str() || !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let Some(mut segments) = url.path_segments() else {
            continue;
        };
        let Some(section) = segments.next() else {
            continue;
        };
        if segments.next().is_none() || ignored_section(section) {
            continue;
        }
        let title = element
            .text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !(4..=160).contains(&title.len()) {
            continue;
        }
        by_prefix
            .entry(section.to_string())
            .or_default()
            .insert(url.to_string(), title);
    }
    let (_, links) = by_prefix.into_iter().max_by_key(|(_, links)| links.len())?;
    if links.len() < 3 {
        return None;
    }
    let mut links: Vec<(String, String)> = links.into_iter().collect();
    links.sort();

    let title = Html::parse_document(html)
        .select(&Selector::parse("title").ok()?)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| base.host_str().unwrap_or("Feed").to_string());
    synthetic_feed(&title, base, links)
}

/// Sites without RSS often still publish a sitemap. Use it as a last resort so
/// the feed is not empty; entries only carry a slug-derived title.
async fn sitemap_feed(client: &Client, base: &Url) -> Option<feed_rs::model::Feed> {
    let mut locs = Vec::new();
    for path in ["/sitemap.xml", "/sitemap_index.xml"] {
        let url = base.join(path).ok()?;
        let Ok(fetched) = ssrf::get(client, &url, &HeaderMap::new()).await else {
            continue;
        };
        if !fetched.status.is_success() {
            continue;
        }
        let Ok(xml) = std::str::from_utf8(&fetched.body) else {
            continue;
        };
        let found = sitemap_locs(xml);
        if xml.contains("<sitemapindex") {
            for child in found.iter().take(8) {
                let Ok(url) = Url::parse(child) else {
                    continue;
                };
                if url.host_str() != base.host_str() {
                    continue;
                }
                let Ok(next) = ssrf::get(client, &url, &HeaderMap::new()).await else {
                    continue;
                };
                if let Ok(xml) = std::str::from_utf8(&next.body) {
                    locs.extend(sitemap_locs(xml));
                }
                if locs.len() >= 100 {
                    break;
                }
            }
        } else {
            locs = found;
        }
        if locs.len() >= 3 {
            break;
        }
    }
    let mut links: Vec<(String, String)> = locs
        .into_iter()
        .filter_map(|loc| {
            let url = Url::parse(&loc).ok()?;
            if url.host_str() != base.host_str() {
                return None;
            }
            let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
            if segments.is_empty() || ignored_section(segments[0]) {
                return None;
            }
            let slug = segments.last()?;
            if reserved_page(slug) {
                return None;
            }
            let title = slug.replace(['-', '_'], " ");
            if !(3..=120).contains(&title.len()) {
                return None;
            }
            Some((url.to_string(), title))
        })
        .collect();
    links.sort();
    links.dedup_by(|a, b| a.0 == b.0);
    if links.len() < 3 {
        return None;
    }
    synthetic_feed(base.host_str().unwrap_or("Feed"), base, links)
}

pub(crate) fn sitemap_locs(xml: &str) -> Vec<String> {
    let mut locs = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<loc>") {
        rest = &rest[start + 5..];
        let Some(end) = rest.find("</loc>") else {
            break;
        };
        let loc = rest[..end].trim();
        if loc.starts_with("http") {
            locs.push(loc.to_string());
        }
        rest = &rest[end + 6..];
    }
    locs
}

fn reserved_page(slug: &str) -> bool {
    matches!(
        slug,
        "" | "."
            | "about"
            | "contact"
            | "now"
            | "uses"
            | "projects"
            | "blog"
            | "writing"
            | "posts"
            | "archive"
            | "rss"
            | "feed"
            | "sitemap"
            | "privacy"
            | "terms"
    )
}

fn synthetic_feed(
    title: &str,
    base: &Url,
    links: Vec<(String, String)>,
) -> Option<feed_rs::model::Feed> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{}</title><id>{}</id><updated>{}</updated>",
        xml_escape(title),
        xml_escape(base.as_str()),
        now
    );
    for (url, title) in links.iter().take(100) {
        xml.push_str(&format!(
            "<entry><id>{}</id><title>{}</title><link href=\"{}\"/><updated>{}</updated></entry>",
            xml_escape(url),
            xml_escape(title),
            xml_escape(url),
            now
        ));
    }
    xml.push_str("</feed>");
    feed_rs::parser::parse(xml.as_bytes())
        .ok()
        .filter(|feed| !feed.entries.is_empty())
}

fn ignored_section(section: &str) -> bool {
    matches!(
        section,
        "tag"
            | "tags"
            | "category"
            | "categories"
            | "author"
            | "page"
            | "search"
            | "login"
            | "account"
            | "privacy"
            | "terms"
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn resolve_youtube(client: &Client, input: &Url) -> Result<Resolved, String> {
    let channel_id = match channel_id_from_path(input).or_else(|| channel_id_from_query(input)) {
        Some(id) => id,
        None => channel_id_from_page(client, input).await?,
    };
    // Channel artwork is best-effort; a miss must never fail the submission.
    let images = fetch_channel_images(client, &channel_id).await;
    let feed_url = format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}");
    let url =
        Url::parse(&feed_url).map_err(|_| "That YouTube channel has an invalid id.".to_string())?;
    let mut last_status = None;
    // YouTube occasionally answers 200 with an empty/consent page under load.
    for attempt in 0..3 {
        let fetched = ssrf::get(client, &url, &HeaderMap::new()).await?;
        if fetched.status.is_success() {
            if let Some(feed) = parse_feed(&fetched.body) {
                let mut resolved = build(fetched.final_url, feed, fetched.headers, input, false);
                if let Some(images) = &images {
                    resolved.avatar_url = images.avatar.clone();
                    resolved.banner_url = images.banner.clone();
                    resolved.images_fetched = true;
                }
                return Ok(resolved);
            }
        } else {
            last_status = Some(fetched.status);
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
    }
    // The RSS service can be down while the channel page is fine; synthesize a
    // feed from `/videos` rather than failing the submission.
    if let Some(feed) = fetch_channel_videos(client, &channel_id).await {
        let mut resolved = build(url, feed, HeaderMap::new(), input, true);
        if let Some(images) = &images {
            resolved.avatar_url = images.avatar.clone();
            resolved.banner_url = images.banner.clone();
            resolved.images_fetched = true;
        }
        return Ok(resolved);
    }
    match last_status {
        Some(status) => Err(format!(
            "That YouTube channel's feed returned HTTP {}.",
            status.as_u16()
        )),
        None => Err("That YouTube channel has no public videos.".into()),
    }
}

async fn channel_id_from_page(client: &Client, input: &Url) -> Result<String, String> {
    let url = Url::parse(&format!("https://m.youtube.com{}", input.path()))
        .map_err(|_| "That YouTube URL is invalid.".to_string())?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(YOUTUBE_UA));
    headers.insert(COOKIE, HeaderValue::from_static(YOUTUBE_COOKIE));
    let fetched = ssrf::get(client, &url, &headers).await?;
    if !fetched.status.is_success() {
        return Err(format!(
            "That YouTube page returned HTTP {}.",
            fetched.status.as_u16()
        ));
    }
    let html = std::str::from_utf8(&fetched.body)
        .map_err(|_| "Could not read that YouTube page.".to_string())?;
    extract_channel_id(html).ok_or_else(|| "Could not find that YouTube channel.".to_string())
}

fn is_youtube(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com")
    )
}

/// Channel artwork lifted from a channel page's `ytInitialData`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelImages {
    pub avatar: Option<String>,
    pub banner: Option<String>,
}

/// Best-effort: a failed fetch or missing artwork is not an error.
pub async fn fetch_channel_images(client: &Client, channel_id: &str) -> Option<ChannelImages> {
    let url = Url::parse(&format!("https://www.youtube.com/channel/{channel_id}")).ok()?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(YOUTUBE_DESKTOP_UA));
    headers.insert(COOKIE, HeaderValue::from_static(YOUTUBE_COOKIE));
    let fetched = ssrf::get(client, &url, &headers).await.ok()?;
    if !fetched.status.is_success() {
        return None;
    }
    let html = std::str::from_utf8(&fetched.body).ok()?;
    Some(extract_channel_images(html))
}

pub fn extract_channel_images(html: &str) -> ChannelImages {
    let Some(data) = initial_data(html) else {
        return ChannelImages::default();
    };
    let avatar = data
        .pointer("/metadata/channelMetadataRenderer/avatar/thumbnails")
        .and_then(largest_image);
    // YouTube moved banners from `banner.thumbnails` to
    // `banner.imageBannerViewModel.image.sources`; accept both.
    let banner = find_key_image(&data, "banner");
    ChannelImages {
        avatar: avatar.as_deref().and_then(safe_image_url),
        banner: banner.as_deref().and_then(safe_image_url),
    }
}

/// The site's own social-preview image, used as the feed's cover. Resolved
/// against the page it came from (final URL after redirects).
pub fn extract_og_image(html: &str, base: &Url) -> Option<String> {
    let selector = Selector::parse(
        "meta[property='og:image'], meta[name='og:image'], \
         meta[name='twitter:image'], meta[property='twitter:image']",
    )
    .ok()?;
    let document = Html::parse_document(html);
    document
        .select(&selector)
        .filter_map(|element| element.value().attr("content"))
        .find_map(|content| {
            let url = base.join(content.trim()).ok()?;
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
        })
}

/// One video lifted from a channel's `/videos` page.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelVideo {
    pub id: String,
    pub title: String,
    pub published: Option<DateTime<Utc>>,
}

/// Fallback for when the channel's RSS feed is down: the `/videos` page is a
/// separate service and usually keeps working. Best-effort, `None` on failure.
pub async fn fetch_channel_videos(
    client: &Client,
    channel_id: &str,
) -> Option<feed_rs::model::Feed> {
    let url = Url::parse(&format!(
        "https://www.youtube.com/channel/{channel_id}/videos"
    ))
    .ok()?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(YOUTUBE_DESKTOP_UA));
    headers.insert(COOKIE, HeaderValue::from_static(YOUTUBE_COOKIE));
    let fetched = ssrf::get(client, &url, &headers).await.ok()?;
    if !fetched.status.is_success() {
        return None;
    }
    let html = std::str::from_utf8(&fetched.body).ok()?;
    let videos = extract_channel_videos(html);
    if videos.is_empty() {
        return None;
    }
    let title = page_title(html).unwrap_or_else(|| "YouTube".to_string());
    parse_feed(&channel_video_feed(channel_id, &title, &videos))
}

/// The channel name as the page's social title.
fn page_title(html: &str) -> Option<String> {
    let selector = Selector::parse("meta[property='og:title']").ok()?;
    let document = Html::parse_document(html);
    document
        .select(&selector)
        .next()?
        .value()
        .attr("content")
        .map(|content| content.trim().to_string())
        .filter(|title| !title.is_empty())
}

pub fn extract_channel_videos(html: &str) -> Vec<ChannelVideo> {
    let Some(data) = initial_data(html) else {
        return Vec::new();
    };
    let mut videos = Vec::new();
    collect_channel_videos(&data, &mut videos);
    let mut seen = std::collections::HashSet::new();
    videos.retain(|video| seen.insert(video.id.clone()));
    videos
}

fn collect_channel_videos(value: &serde_json::Value, out: &mut Vec<ChannelVideo>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(video) = map.get("lockupViewModel").and_then(channel_video) {
                out.push(video);
            }
            for child in map.values() {
                collect_channel_videos(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_channel_videos(item, out);
            }
        }
        _ => {}
    }
}

fn channel_video(lockup: &serde_json::Value) -> Option<ChannelVideo> {
    let id = lockup.get("contentId")?.as_str()?;
    if id.len() != 11 || !id.chars().all(is_channel_char) {
        return None;
    }
    let title = lockup
        .pointer("/metadata/lockupMetadataViewModel/title/content")?
        .as_str()?
        .trim();
    if title.is_empty() {
        return None;
    }
    let published = find_relative_time(lockup)
        .and_then(|text| relative_seconds(&text))
        .map(|seconds| Utc::now() - Duration::seconds(seconds));
    Some(ChannelVideo {
        id: id.to_string(),
        title: title.to_string(),
        published,
    })
}

fn find_relative_time(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(text) = map
                .get("text")
                .and_then(|text| text.get("content"))
                .and_then(serde_json::Value::as_str)
                && relative_seconds(text).is_some()
            {
                return Some(text.to_string());
            }
            map.values().find_map(find_relative_time)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_relative_time),
        _ => None,
    }
}

/// "3 days ago" / "Streamed 2 weeks ago" → seconds before now.
fn relative_seconds(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text
        .strip_prefix("Streamed ")
        .or_else(|| text.strip_prefix("Premiered "))
        .unwrap_or(text);
    let mut parts = text.strip_suffix(" ago")?.split_whitespace();
    let count: i64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.trim_end_matches('s');
    let seconds = match unit {
        // YouTube's channel page abbreviates older dates: "1 yr ago".
        "second" | "sec" => 1,
        "minute" | "min" => 60,
        "hour" | "hr" => 3600,
        "day" => 86_400,
        "week" | "wk" => 604_800,
        "month" | "mo" => 2_592_000,
        "year" | "yr" => 31_536_000,
        _ => return None,
    };
    Some(count * seconds)
}

/// Atom document shaped like the RSS endpoint's, so `parse_feed` and `ingest`
/// handle both identically.
fn channel_video_feed(channel_id: &str, channel_title: &str, videos: &[ChannelVideo]) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\">",
    );
    xml.push_str(&format!(
        "<id>channel:{channel_id}</id><title>{}</title><updated>{}</updated>",
        xml_escape(channel_title),
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    ));
    for video in videos {
        xml.push_str("<entry>");
        xml.push_str(&format!("<id>yt:video:{}</id>", video.id));
        xml.push_str(&format!("<title>{}</title>", xml_escape(&video.title)));
        xml.push_str(&format!(
            "<link rel=\"alternate\" href=\"https://www.youtube.com/watch?v={}\"/>",
            video.id
        ));
        if let Some(published) = video.published {
            xml.push_str(&format!(
                "<published>{}</published>",
                published.to_rfc3339_opts(SecondsFormat::Secs, true)
            ));
        }
        xml.push_str("</entry>");
    }
    xml.push_str("</feed>");
    xml.into_bytes()
}

/// The channel id embedded in a YouTube RSS feed URL.
pub fn channel_id_from_feed_url(feed_url: &str) -> Option<String> {
    let url = Url::parse(feed_url).ok()?;
    is_youtube(&url)
        .then(|| channel_id_from_query(&url))
        .flatten()
}

fn initial_data(html: &str) -> Option<serde_json::Value> {
    let marker = html.find("ytInitialData")?;
    let start = marker + html[marker..].find('{')?;
    let end = json_object_end(html.as_bytes(), start)?;
    serde_json::from_str(&html[start..end]).ok()
}

/// End offset (exclusive) of the JSON object starting at `start`.
fn json_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Picks the widest `url` anywhere below `value`.
fn largest_image(value: &serde_json::Value) -> Option<String> {
    fn walk(value: &serde_json::Value, best: &mut Option<(i64, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(url) = map.get("url").and_then(serde_json::Value::as_str) {
                    let width = map
                        .get("width")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    if best.as_ref().is_none_or(|(best, _)| width >= *best) {
                        *best = Some((width, url.to_string()));
                    }
                }
                for child in map.values() {
                    walk(child, best);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, best);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    walk(value, &mut best);
    best.map(|(_, url)| url)
}

fn find_key_image(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(url) = map.get(key).and_then(largest_image) {
                return Some(url);
            }
            map.values().find_map(|child| find_key_image(child, key))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|item| find_key_image(item, key)),
        _ => None,
    }
}

/// Only YouTube's own image CDN is allowed into the templates.
fn safe_image_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https" && url.host_str() == Some("yt3.googleusercontent.com"))
        .then(|| url.to_string())
}

fn channel_id_from_path(url: &Url) -> Option<String> {
    let mut segments = url.path_segments()?;
    if segments.next()? != "channel" {
        return None;
    }
    let id = segments.next()?;
    is_channel_id(id).then(|| id.to_string())
}

fn channel_id_from_query(url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(key, value)| key == "channel_id" && is_channel_id(value))
        .map(|(_, value)| value.into_owned())
}

pub(crate) fn extract_channel_id(html: &str) -> Option<String> {
    for marker in [r#""channelId":""#, r#""externalId":""#, r#""browseId":""#] {
        if let Some(id) = marker_id(html, marker) {
            return Some(id);
        }
    }
    if let Some(id) = channel_path_id(html) {
        return Some(id);
    }
    itemprop_id(html)
}

fn marker_id(html: &str, marker: &str) -> Option<String> {
    html.match_indices(marker).find_map(|(index, _)| {
        let rest = &html[index + marker.len()..];
        let end = rest
            .find(|c: char| !is_channel_char(c))
            .unwrap_or(rest.len());
        let id = &rest[..end];
        is_channel_id(id).then(|| id.to_string())
    })
}

fn itemprop_id(html: &str) -> Option<String> {
    html.match_indices(r#"itemprop="identifier""#)
        .find_map(|(index, _)| {
            let rest = &html[index..];
            let start = rest.find("content=\"")? + "content=\"".len();
            let rest = &rest[start..];
            let end = rest.find('"').unwrap_or(rest.len());
            let id = &rest[..end];
            is_channel_id(id).then(|| id.to_string())
        })
}

fn channel_path_id(html: &str) -> Option<String> {
    html.match_indices("/channel/").find_map(|(index, _)| {
        let rest = &html[index + "/channel/".len()..];
        let end = rest
            .find(|c: char| !is_channel_char(c))
            .unwrap_or(rest.len());
        let id = &rest[..end];
        is_channel_id(id).then(|| id.to_string())
    })
}

fn is_channel_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

pub fn parse_feed(body: &[u8]) -> Option<feed_rs::model::Feed> {
    let feed = feed_rs::parser::parse(body)
        .or_else(|_| feed_rs::parser::parse(remove_empty_elements(body).as_slice()))
        .ok()?;
    if feed.entries.is_empty() {
        return None;
    }
    Some(feed)
}

/// feed-rs rejects text elements with no content (common in YouTube feeds),
/// so drop exact empty pairs before the fallback parse.
fn remove_empty_elements(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        if body[index] != b'<' {
            out.push(body[index]);
            index += 1;
            continue;
        }
        let Some(offset) = body[index..].iter().position(|&b| b == b'>') else {
            out.extend_from_slice(&body[index..]);
            break;
        };
        let end = index + offset;
        let tag = &body[index + 1..end];
        let starts_name = tag
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || matches!(*b, b'_' | b':'));
        if starts_name && tag.last() != Some(&b'/') {
            let name_end = tag
                .iter()
                .position(|&b| b.is_ascii_whitespace() || b == b'/')
                .unwrap_or(tag.len());
            let name = &tag[..name_end];
            if let Some(close) = empty_close(body, end + 1, name) {
                index = close;
                continue;
            }
        }
        out.extend_from_slice(&body[index..=end]);
        index = end + 1;
    }
    out
}

fn empty_close(body: &[u8], after: usize, name: &[u8]) -> Option<usize> {
    let rest = body.get(after..)?;
    let rest = rest.strip_prefix(b"</")?;
    if !rest.starts_with(name) || rest.get(name.len()) != Some(&b'>') {
        return None;
    }
    Some(after + 2 + name.len() + 1)
}

fn build(
    feed_url: Url,
    feed: feed_rs::model::Feed,
    headers: HeaderMap,
    input: &Url,
    synthetic: bool,
) -> Resolved {
    let site_url = feed
        .entries
        .iter()
        .flat_map(|entry| entry.links.iter())
        .find_map(|link| Url::parse(&link.href).ok())
        .and_then(|url| url.origin().ascii_serialization().parse::<Url>().ok())
        .or_else(|| input.origin().ascii_serialization().parse::<Url>().ok())
        .unwrap_or_else(|| input.clone());

    let title = feed
        .title
        .as_ref()
        .map(|t| t.content.trim().to_string())
        .filter(|t| !t.is_empty());
    let description = feed
        .description
        .as_ref()
        .map(|t| t.content.trim().to_string())
        .filter(|t| !t.is_empty());

    Resolved {
        feed_url: feed_url.to_string(),
        site_url: site_url.to_string(),
        title,
        description,
        avatar_url: None,
        banner_url: None,
        images_fetched: false,
        feed,
        etag: ssrf::header_value(&headers, ETAG),
        last_modified: ssrf::header_value(&headers, LAST_MODIFIED),
        synthetic,
    }
}

fn alternates(html: &str, base: &Url) -> Vec<Url> {
    let Ok(selector) = Selector::parse(r#"link[rel~="alternate"]"#) else {
        return Vec::new();
    };
    let document = Html::parse_document(html);
    document
        .select(&selector)
        .filter(|el| {
            matches!(
                el.value().attr("type"),
                Some(
                    "application/rss+xml"
                        | "application/atom+xml"
                        | "application/feed+json"
                        | "application/json"
                )
            )
        })
        .filter_map(|el| el.value().attr("href"))
        .filter_map(|href| base.join(href).ok())
        .collect()
}

fn is_channel_id(value: &str) -> bool {
    value.starts_with("UC") && value.len() == 24 && value.chars().all(is_channel_char)
}

#[cfg(test)]
mod tests {
    use super::{ChannelVideo, channel_video_feed, parse_feed};
    use chrono::Utc;

    #[test]
    fn synthesized_channel_feed_parses_like_rss() {
        let videos = vec![ChannelVideo {
            id: "dQw4w9WgXcQ".into(),
            title: "A & B".into(),
            published: Some(Utc::now()),
        }];
        let feed = parse_feed(&channel_video_feed(
            "UCfQx-K9zBExhMLfKQmXBxJg",
            "Sam & Victor",
            &videos,
        ))
        .expect("synthesized atom must parse");
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].id, "yt:video:dQw4w9WgXcQ");
        assert_eq!(feed.entries[0].title.as_ref().unwrap().content, "A & B");
    }
}
