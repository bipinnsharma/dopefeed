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
    pub post_count: i64,
    pub has_new_post: i64,
    pub latest_label: Option<String>,
}

impl Feed {
    pub fn is_new(&self) -> bool {
        self.has_new_post != 0
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
    pub published_label: String,
}

impl Post {
    /// YouTube entries carry a `yt:video:<id>` guid; used for the privacy embed.
    pub fn youtube_id(&self) -> Option<&str> {
        self.guid.strip_prefix("yt:video:").filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    }

    /// Feeds that only publish a link/summary have no readable article body.
    pub fn is_stub(&self) -> bool {
        self.content_html.trim().is_empty()
    }

    /// First image in the entry (its usual OG/hero image), else the YouTube
    /// thumbnail. Used as the detail-page cover.
    pub fn cover_image(&self) -> Option<String> {
        if let Some(id) = self.youtube_id() {
            return Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
        }
        let document = scraper::Html::parse_fragment(&self.content_html);
        let selector = scraper::Selector::parse("img[src]").ok()?;
        let base = url::Url::parse(&self.url).ok();
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
