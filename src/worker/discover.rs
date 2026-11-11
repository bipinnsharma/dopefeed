use super::ssrf;
use reqwest::{
    Client,
    header::{ETAG, HeaderMap, LAST_MODIFIED},
};
use scraper::{Html, Selector};
use url::Url;

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

        if let Some(channel_id) = channel_id(html, &fetched.final_url) {
            let feed_url =
                format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}");
            if let Ok(url) = Url::parse(&feed_url)
                && let Ok(next) = ssrf::get(client, &url, &HeaderMap::new()).await
                && let Some(feed) = parse_feed(&next.body)
            {
                return Ok(build(next.final_url, feed, next.headers, input));
            }
        }
    }

    Err("Could not find a feed at that URL. Try the feed link or a YouTube channel.".into())
}

pub fn parse_feed(body: &[u8]) -> Option<feed_rs::model::Feed> {
    let feed = feed_rs::parser::parse(body).ok()?;
    if feed.entries.is_empty() {
        return None;
    }
    Some(feed)
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

fn channel_id(html: &str, url: &Url) -> Option<String> {
    if !matches!(
        url.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be")
    ) {
        return None;
    }
    if let Some(index) = html.find("\"channelId\":\"UC") {
        let rest = &html[index + "\"channelId\":\"".len()..];
        let id = rest.split('"').next()?;
        if is_channel_id(id) {
            return Some(id.to_string());
        }
    }
    if let Ok(selector) = Selector::parse(r#"link[rel="canonical"]"#) {
        let document = Html::parse_document(html);
        for el in document.select(&selector) {
            let href = el.value().attr("href").unwrap_or_default();
            if let Some(id) = href
                .rsplit("/channel/")
                .next()
                .filter(|id| is_channel_id(id))
            {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn is_channel_id(value: &str) -> bool {
    value.starts_with("UC")
        && value.len() >= 20
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
