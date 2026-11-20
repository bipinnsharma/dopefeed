use crate::{
    error::{AppError, AppResult},
    models::{Feed, FeedQuery, Post, PostRow, TIMELINE_PAGE},
    state::{AppState, POST_COLUMNS, POST_READER_COLUMNS},
};
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Html,
};

#[derive(Template)]
#[template(path = "feed.html")]
struct FeedTemplate {
    feed: Feed,
    posts: Vec<PostRow>,
    flash: Option<String>,
    q: String,
    page: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "_posts_region.html")]
struct PostRegion {
    feed: Feed,
    posts: Vec<PostRow>,
    q: String,
    page: i64,
    has_more: bool,
}

struct FeedPosts {
    feed: Feed,
    posts: Vec<PostRow>,
    q: String,
    page: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "reader.html")]
struct ReaderTemplate {
    post: Post,
    cover: Option<String>,
    youtube_id: Option<String>,
}

const FEED_SQL: &str = "SELECT f.id, f.site_url, f.domain, f.title, f.description, f.category,
         f.avatar_url, f.banner_url,
         date((SELECT MAX(p.published_at) FROM posts p WHERE p.feed_id = f.id)) AS latest_label,
         (SELECT MAX(p.published_at) FROM posts p WHERE p.feed_id = f.id) AS latest_published_at,
         (SELECT COUNT(*) FROM posts p WHERE p.feed_id = f.id) AS post_count,
         EXISTS(SELECT 1 FROM posts p WHERE p.feed_id = f.id
                AND p.published_at >= datetime('now', '-48 hours')) AS has_new_post
         FROM feeds f WHERE f.id = ? AND f.status = 'active'";

async fn load_feed_posts(state: &AppState, id: i64, query: &FeedQuery) -> AppResult<FeedPosts> {
    let feed = sqlx::query_as::<_, Feed>(FEED_SQL)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;

    let q = query.q.as_deref().unwrap_or("").trim().to_string();
    // A search shows the newest matches and spans the feed; paging only
    // applies to the unfiltered list.
    let last_page = ((feed.post_count - 1).max(0) / TIMELINE_PAGE) + 1;
    let page = if q.is_empty() {
        query.page.unwrap_or(1).max(1).min(last_page)
    } else {
        1
    };
    let posts = sqlx::query_as::<_, PostRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLUMNS} FROM posts p JOIN feeds f ON f.id = p.feed_id
         WHERE p.feed_id = ? AND p.title LIKE ?
         ORDER BY p.published_at DESC, p.id DESC LIMIT ? OFFSET ?"
    )))
    .bind(id)
    .bind(format!("%{q}%"))
    .bind(TIMELINE_PAGE)
    .bind((page - 1) * TIMELINE_PAGE)
    .fetch_all(&state.pool)
    .await?;
    let has_more = q.is_empty() && page < last_page;
    Ok(FeedPosts {
        feed,
        posts,
        q,
        page,
        has_more,
    })
}

pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FeedQuery>,
) -> AppResult<Html<String>> {
    let view = load_feed_posts(&state, id, &query).await?;
    let flash = match query.msg.as_deref() {
        Some("added") => Some("Feed added. Posts will appear as they are fetched.".to_string()),
        _ => None,
    };

    Ok(Html(
        FeedTemplate {
            feed: view.feed,
            posts: view.posts,
            flash,
            q: view.q,
            page: view.page,
            has_more: view.has_more,
        }
        .render()?,
    ))
}

/// htmx fragment for the in-feed search: just the post list region.
pub async fn posts(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FeedQuery>,
) -> AppResult<Html<String>> {
    let view = load_feed_posts(&state, id, &query).await?;
    Ok(Html(
        PostRegion {
            feed: view.feed,
            posts: view.posts,
            q: view.q,
            page: view.page,
            has_more: view.has_more,
        }
        .render()?,
    ))
}

pub async fn read(State(state): State<AppState>, Path(id): Path<i64>) -> AppResult<Html<String>> {
    let post = sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_READER_COLUMNS} FROM posts p JOIN feeds f ON f.id = p.feed_id
         WHERE p.id = ? AND f.status = 'active'"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let youtube_id = post.youtube_id().map(str::to_string);
    let cover = post.reader_cover();
    Ok(Html(
        ReaderTemplate {
            post,
            cover,
            youtube_id,
        }
        .render()?,
    ))
}
