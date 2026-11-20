use crate::{
    error::AppResult,
    models::{DirectoryQuery, Feed, FeedPage, PAGE_SIZE},
    state::AppState,
};
use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

#[derive(Template)]
#[template(path = "directory.html")]
pub struct DirectoryPage {
    feeds: Vec<Feed>,
    total: i64,
    shown: i64,
    category: String,
    q: String,
    sort: String,
    page: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "_feed_grid.html")]
pub struct FeedGrid {
    feeds: Vec<Feed>,
    total: i64,
    shown: i64,
    category: String,
    q: String,
    sort: String,
    page: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "_feed_grid_more.html")]
pub struct FeedGridMore {
    feeds: Vec<Feed>,
    category: String,
    q: String,
    sort: String,
    page: i64,
    has_more: bool,
}

pub async fn index(
    State(state): State<AppState>,
    Query(query): Query<DirectoryQuery>,
) -> AppResult<Html<String>> {
    let page = load(&state.pool, &query).await?;
    let html = DirectoryPage {
        feeds: page.feeds,
        total: page.total,
        shown: page.shown,
        category: page.category,
        q: page.q,
        sort: page.sort,
        page: page.page,
        has_more: page.has_more,
    }
    .render()?;
    Ok(Html(html))
}

pub async fn grid(
    State(state): State<AppState>,
    Query(query): Query<DirectoryQuery>,
) -> AppResult<Html<String>> {
    let more = query.more.is_some() && query.page.unwrap_or(1) > 1;
    let page = load(&state.pool, &query).await?;
    let html = if more {
        FeedGridMore {
            feeds: page.feeds,
            category: page.category,
            q: page.q,
            sort: page.sort,
            page: page.page,
            has_more: page.has_more,
        }
        .render()?
    } else {
        FeedGrid {
            feeds: page.feeds,
            total: page.total,
            shown: page.shown,
            category: page.category,
            q: page.q,
            sort: page.sort,
            page: page.page,
            has_more: page.has_more,
        }
        .render()?
    };
    Ok(Html(html))
}

async fn load(pool: &SqlitePool, query: &DirectoryQuery) -> AppResult<FeedPage> {
    let category = query
        .category
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let category = if crate::models::CATEGORIES.contains(&category.as_str()) {
        category
    } else {
        String::new()
    };
    let q = query
        .q
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(100)
        .collect::<String>();
    let sort = match query.sort.as_deref() {
        Some("popular") => "popular",
        Some("oldest") => "oldest",
        _ => "recent",
    }
    .to_string();
    let page = query.page.unwrap_or(1).max(1);

    let mut count =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM feeds f WHERE f.status = 'active'");
    push_filters(&mut count, &category, &q);
    let total = count.build_query_scalar::<i64>().fetch_one(pool).await?;

    let mut feeds = QueryBuilder::<Sqlite>::new(
        "SELECT f.id, f.site_url, f.domain, f.title, f.description, f.category,
         f.avatar_url, f.banner_url,
         date((SELECT MAX(p.published_at) FROM posts p WHERE p.feed_id = f.id)) AS latest_label,
         (SELECT MAX(p.published_at) FROM posts p WHERE p.feed_id = f.id) AS latest_published_at,
         (SELECT COUNT(*) FROM posts p WHERE p.feed_id = f.id) AS post_count,
         EXISTS(SELECT 1 FROM posts p WHERE p.feed_id = f.id
                AND p.published_at >= datetime('now', '-48 hours')) AS has_new_post
         FROM feeds f WHERE f.status = 'active'",
    );
    push_filters(&mut feeds, &category, &q);
    let order = match sort.as_str() {
        "popular" => " ORDER BY post_count DESC, f.created_at DESC, f.id DESC LIMIT ",
        "oldest" => " ORDER BY f.created_at ASC, f.id ASC LIMIT ",
        _ => {
            " ORDER BY (SELECT MAX(p.published_at) FROM posts p WHERE p.feed_id = f.id) DESC,
             f.created_at DESC, f.id DESC LIMIT "
        }
    };
    feeds
        .push(order)
        .push_bind(PAGE_SIZE)
        .push(" OFFSET ")
        .push_bind((page - 1) * PAGE_SIZE);
    let feeds = feeds.build_query_as::<Feed>().fetch_all(pool).await?;

    Ok(FeedPage {
        feeds,
        total,
        shown: (page * PAGE_SIZE).min(total),
        category,
        q,
        sort,
        page,
        has_more: page * PAGE_SIZE < total,
    })
}

/// Values are always bound; only the fixed filter fragments are dynamic SQL.
fn push_filters(query: &mut QueryBuilder<Sqlite>, category: &str, q: &str) {
    if !category.is_empty() {
        query.push(" AND f.category = ").push_bind(category);
    }
    if !q.is_empty() {
        let like = format!("%{q}%");
        query
            .push(" AND (f.title LIKE ")
            .push_bind(like.clone())
            .push(" OR f.description LIKE ")
            .push_bind(like.clone())
            .push(" OR f.domain LIKE ")
            .push_bind(like)
            .push(")");
    }
}
