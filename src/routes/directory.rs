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
use sqlx::SqlitePool;

#[derive(Template)]
#[template(path = "directory.html")]
pub struct DirectoryPage {
    feeds: Vec<Feed>,
    total: i64,
    shown: i64,
    category: String,
    q: String,
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
    page: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "_feed_grid_more.html")]
pub struct FeedGridMore {
    feeds: Vec<Feed>,
    category: String,
    q: String,
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
    let page = query.page.unwrap_or(1).max(1);

    let mut filters = vec!["f.status = 'active'".to_string()];
    let mut bindings: Vec<String> = Vec::new();
    if !category.is_empty() {
        filters.push("f.category = ?".into());
        bindings.push(category.clone());
    }
    if !q.is_empty() {
        filters.push("(f.title LIKE ? OR f.description LIKE ? OR f.domain LIKE ?)".into());
        let like = format!("%{q}%");
        bindings.extend([like.clone(), like.clone(), like]);
    }
    let where_sql = filters.join(" AND ");

    let mut count_query = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM feeds f WHERE {where_sql}"
    )));
    for value in &bindings {
        count_query = count_query.bind(value);
    }
    let total = count_query.fetch_one(pool).await?;

    let mut feeds_query = sqlx::query_as::<_, Feed>(sqlx::AssertSqlSafe(format!(
        "SELECT f.id, f.site_url, f.domain, f.title, f.description, f.category,
         strftime('%b %d, %Y', f.last_fetched_at) AS fetched_label,
         (SELECT COUNT(*) FROM posts p WHERE p.feed_id = f.id) AS post_count,
         EXISTS(SELECT 1 FROM posts p WHERE p.feed_id = f.id
                AND p.published_at >= datetime('now', '-48 hours')) AS has_new_post
         FROM feeds f WHERE {where_sql}
         ORDER BY f.created_at DESC, f.id DESC LIMIT ? OFFSET ?"
    )));
    for value in &bindings {
        feeds_query = feeds_query.bind(value);
    }
    let feeds = feeds_query
        .bind(PAGE_SIZE)
        .bind((page - 1) * PAGE_SIZE)
        .fetch_all(pool)
        .await?;

    Ok(FeedPage {
        feeds,
        total,
        shown: (page * PAGE_SIZE).min(total),
        category,
        q,
        page,
        has_more: page * PAGE_SIZE < total,
    })
}
