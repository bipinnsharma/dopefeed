use super::ssrf;
use reqwest::{
    Client,
    header::{COOKIE, ETAG, HeaderMap, HeaderValue, LAST_MODIFIED, USER_AGENT},
};
use scraper::{Html, Selector};
use url::Url;

const YOUTUBE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
const YOUTUBE_COOKIE: &str = "CONSENT=YES+cb.20210328-17-p0.en+FX+417; SOCS=CAI";

pub struct Resolved {
    pub feed_url: String,
    pub site_url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub feed: feed_rs::model::Feed,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
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
        return Ok(build(fetched.final_url, feed, fetched.headers, input));
    }

    if let Ok(html) = std::str::from_utf8(&fetched.body) {
        for candidate in alternates(html, &fetched.final_url) {
            let Ok(next) = ssrf::get(client, &candidate, &HeaderMap::new()).await else {
                continue;
            };
            if let Some(feed) = parse_feed(&next.body) {
                return Ok(build(next.final_url, feed, next.headers, input));
            }
        }
    }

    Err("Could not find a feed at that URL. Try the feed link or a YouTube channel.".into())
}

async fn resolve_youtube(client: &Client, input: &Url) -> Result<Resolved, String> {
    let channel_id = match channel_id_from_path(input).or_else(|| channel_id_from_query(input)) {
        Some(id) => id,
        None => channel_id_from_page(client, input).await?,
    };
    let feed_url = format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}");
    let url =
        Url::parse(&feed_url).map_err(|_| "That YouTube channel has an invalid id.".to_string())?;
    let mut last_status = None;
    // YouTube occasionally answers 200 with an empty/consent page under load.
    for attempt in 0..3 {
        let fetched = ssrf::get(client, &url, &HeaderMap::new()).await?;
        if fetched.status.is_success() {
            if let Some(feed) = parse_feed(&fetched.body) {
                return Ok(build(fetched.final_url, feed, fetched.headers, input));
            }
        } else {
            last_status = Some(fetched.status);
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
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

fn build(feed_url: Url, feed: feed_rs::model::Feed, headers: HeaderMap, input: &Url) -> Resolved {
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
        feed,
        etag: ssrf::header_value(&headers, ETAG),
        last_modified: ssrf::header_value(&headers, LAST_MODIFIED),
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
                Some("application/rss+xml" | "application/atom+xml")
            )
        })
        .filter_map(|el| el.value().attr("href"))
        .filter_map(|href| base.join(href).ok())
        .collect()
}

fn is_channel_id(value: &str) -> bool {
    value.starts_with("UC") && value.len() == 24 && value.chars().all(is_channel_char)
}
