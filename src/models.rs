use crate::assets;
use serde::Deserialize;

pub const PAGE_SIZE: i64 = 24;
pub const TIMELINE_PAGE: i64 = 50;
pub const CATEGORIES: &[&str] = &[
    "ai",
    "ceo",
    "coding",
    "design",
    "dev",
    "lifestyle",
    "product",
    "startup",
    "tech",
    "writing",
    "youtube",
];

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Feed {
    pub id: i64,
    pub site_url: String,
    pub domain: String,
    pub title: String,
    pub description: Option<String>,
    pub category: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub post_count: i64,
    pub has_new_post: i64,
    pub latest_label: Option<String>,
    pub latest_published_at: Option<String>,
}

impl Feed {
    pub fn is_new(&self) -> bool {
        self.has_new_post != 0
    }

    pub fn formatted_post_count(&self) -> String {
        let digits = self.post_count.to_string();
        let first_group = digits.len() % 3;
        let mut formatted = digits[..first_group].to_string();
        for (index, digit) in digits[first_group..].chars().enumerate() {
            if index % 3 == 0 {
                formatted.push(',');
            }
            formatted.push(digit);
        }
        if first_group == 0 {
            formatted.remove(0);
        }
        format!(
            "{} {}",
            formatted,
            if self.post_count == 1 {
                "post"
            } else {
                "posts"
            }
        )
    }

    pub fn latest_age_label(&self) -> Option<String> {
        use chrono::{DateTime, Duration, NaiveDateTime, Utc};

        let raw = self.latest_published_at.as_deref()?;
        let published = DateTime::parse_from_rfc3339(raw)
            .map(|date| date.with_timezone(&Utc))
            .or_else(|_| {
                NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").map(|date| date.and_utc())
            })
            .ok()?;
        let age = (Utc::now() - published).max(Duration::zero());
        let minutes = age.num_minutes();
        if minutes < 1 {
            Some("just now".into())
        } else if minutes < 60 {
            Some(format!(
                "{minutes} min{} ago",
                if minutes == 1 { "" } else { "s" }
            ))
        } else if age.num_hours() < 24 {
            let hours = age.num_hours();
            Some(format!(
                "{hours} hr{} ago",
                if hours == 1 { "" } else { "s" }
            ))
        } else if age < Duration::days(7) {
            let days = age.num_days();
            Some(format!(
                "{days} day{} ago",
                if days == 1 { "" } else { "s" }
            ))
        } else {
            self.latest_label.clone()
        }
    }

    /// Templates render these, so Blob-hosted assets get their absolute URL.
    pub fn avatar(&self) -> Option<String> {
        self.avatar_url.as_deref().map(assets::url)
    }

    pub fn banner(&self) -> Option<String> {
        self.banner_url.as_deref().map(assets::url)
    }

    pub fn category_label(&self) -> &'static str {
        match self.category.as_str() {
            "ai" => "AI",
            "ceo" => "CEO",
            "coding" => "Coding",
            "design" => "Design",
            "dev" => "Dev",
            "lifestyle" => "Lifestyle",
            "product" => "Product",
            "startup" => "Startups",
            "tech" => "Tech",
            "writing" => "Writing",
            "youtube" => "YouTube",
            _ => "Other",
        }
    }
}

/// A post list row: everything the list templates need, without the body.
#[derive(Clone, Debug, sqlx::FromRow)]
#[allow(dead_code)] // shared column set; a list template reads only some fields
pub struct PostRow {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub feed_domain: String,
    pub guid: String,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub summary: String,
    pub thumb_url: Option<String>,
    pub published_label: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Post {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub feed_domain: String,
    pub guid: String,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub summary: String,
    pub content_html: String,
    pub thumb_url: Option<String>,
    pub published_label: String,
}

pub const PLACEHOLDER_IMAGE: &str = "/static/no-image-dl.jpg";

/// YouTube entries carry a `yt:video:<id>` guid.
pub fn youtube_video_id(guid: &str) -> Option<&str> {
    guid.strip_prefix("yt:video:").filter(|id| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

/// 16:9 poster for a YouTube video, derived from the guid.
pub fn youtube_thumb(guid: &str) -> Option<String> {
    youtube_video_id(guid).map(|id| format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))
}

/// First http(s) image in an article body, resolved against its page URL.
pub fn first_image(html: &str, page_url: &str) -> Option<String> {
    let document = scraper::Html::parse_fragment(html);
    let selector = scraper::Selector::parse("img[src]").ok()?;
    let base = url::Url::parse(page_url).ok();
    document
        .select(&selector)
        .filter_map(|element| element.value().attr("src"))
        .find_map(|src| {
            let src = src.trim();
            let url = base
                .as_ref()
                .and_then(|base| base.join(src).ok())
                .or_else(|| url::Url::parse(src).ok())?;
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
        })
}

impl PostRow {
    /// The image the list row shows: the locally cached one once downloaded.
    pub fn thumb(&self) -> Option<String> {
        self.thumb_url
            .clone()
            .filter(|url| !url.is_empty())
            .or_else(|| youtube_thumb(&self.guid))
    }

    /// The row always has a thumb slot, so a missing poster must not collapse it.
    pub fn thumb_or_placeholder(&self) -> String {
        self.thumb()
            .map(|thumb| assets::url(&thumb))
            .unwrap_or_else(|| PLACEHOLDER_IMAGE.to_string())
    }
}

impl Post {
    /// YouTube id, used for the privacy embed.
    pub fn youtube_id(&self) -> Option<&str> {
        youtube_video_id(&self.guid)
    }

    /// Feeds that only publish a link/summary have no readable article body.
    pub fn is_stub(&self) -> bool {
        self.content_html.trim().is_empty()
    }

    /// First image in the entry (its usual OG/hero image), else the YouTube
    /// thumbnail.
    #[allow(dead_code)] // content-dependent; no request path needs it now
    pub fn cover_image(&self) -> Option<String> {
        if let Some(id) = self.youtube_id() {
            return Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
        }
        first_image(&self.content_html, &self.url)
    }

    /// Reader cover: the cached copy for blog posts, the sharper remote poster
    /// for YouTube. No body parsing: the worker already extracted the thumb.
    pub fn reader_cover(&self) -> Option<String> {
        if let Some(id) = self.youtube_id() {
            return Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
        }
        let thumb = self.thumb_url.clone().filter(|url| !url.is_empty())?;
        Some(assets::url(&cover_variant(&thumb)))
    }

    /// Fallback for a cover whose 1200px sibling is missing.
    pub fn thumb_or_placeholder(&self) -> String {
        self.thumb_url
            .clone()
            .filter(|url| !url.is_empty())
            .map(|thumb| assets::url(&thumb))
            .unwrap_or_else(|| PLACEHOLDER_IMAGE.to_string())
    }
}

/// Cached post images are written next to a 1200px JPEG hero.
fn cover_variant(url: &str) -> String {
    if !url.starts_with("/static/assets/posts/") {
        return url.to_string();
    }
    match url.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}-1200.jpg"),
        None => url.to_string(),
    }
}

#[derive(Debug, Deserialize)]
pub struct DirectoryQuery {
    pub category: Option<String>,
    pub q: Option<String>,
    pub page: Option<i64>,
    pub more: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub feeds: Option<String>,
    pub page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    pub msg: Option<String>,
    pub page: Option<i64>,
    pub q: Option<String>,
}

pub struct FeedPage {
    pub feeds: Vec<Feed>,
    pub total: i64,
    pub shown: i64,
    pub category: String,
    pub q: String,
    pub sort: String,
    pub page: i64,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::{Feed, Post};
    use chrono::{Duration, Utc};

    fn feed(count: i64, published_at: Option<String>) -> Feed {
        Feed {
            id: 1,
            site_url: "https://example.com".into(),
            domain: "example.com".into(),
            title: "Example".into(),
            description: None,
            category: "tech".into(),
            avatar_url: None,
            banner_url: None,
            post_count: count,
            has_new_post: 0,
            latest_label: Some("2025-01-01".into()),
            latest_published_at: published_at,
        }
    }

    #[test]
    fn feed_metadata_formats_count_and_published_age() {
        assert_eq!(feed(2483, None).formatted_post_count(), "2,483 posts");
        assert_eq!(feed(1, None).formatted_post_count(), "1 post");
        assert_eq!(feed(0, None).latest_age_label(), None);
        assert_eq!(feed(1, Some("not a date".into())).latest_age_label(), None);

        let label_for = |age: Duration| {
            let stamp = (Utc::now() - age).format("%Y-%m-%d %H:%M:%S").to_string();
            feed(1, Some(stamp)).latest_age_label().unwrap()
        };
        assert_eq!(label_for(Duration::seconds(10)), "just now");
        assert_eq!(label_for(Duration::minutes(1)), "1 min ago");
        assert_eq!(label_for(Duration::minutes(4)), "4 mins ago");
        assert_eq!(label_for(Duration::hours(1)), "1 hr ago");
        assert_eq!(label_for(Duration::days(2)), "2 days ago");
        assert_eq!(label_for(Duration::days(7)), "2025-01-01");

        let future = (Utc::now() + Duration::days(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert_eq!(
            feed(1, Some(future)).latest_age_label().as_deref(),
            Some("just now")
        );
    }

    fn post(guid: &str, thumb: Option<&str>) -> Post {
        Post {
            id: 1,
            feed_id: 1,
            feed_title: "Feed".into(),
            feed_domain: "example.com".into(),
            guid: guid.into(),
            url: "https://example.com/post".into(),
            title: "Title".into(),
            author: None,
            summary: String::new(),
            content_html: String::new(),
            thumb_url: thumb.map(str::to_string),
            published_label: "2025-01-01".into(),
        }
    }

    #[test]
    fn reader_cover_uses_the_1200px_sibling() {
        assert_eq!(
            post("guid", Some("/static/assets/posts/abc123.png"))
                .reader_cover()
                .as_deref(),
            Some("/static/assets/posts/abc123-1200.jpg")
        );
        assert_eq!(
            post("guid", Some("https://cdn.example.com/hero.webp"))
                .reader_cover()
                .as_deref(),
            Some("https://cdn.example.com/hero.webp")
        );
        assert_eq!(
            post("yt:video:dQw4w9WgXcQ", Some("/static/assets/posts/abc.jpg"))
                .reader_cover()
                .as_deref(),
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
        );
    }
}
