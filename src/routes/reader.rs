use crate::{
    error::{AppError, AppResult},
    models::{Feed, FeedQuery, Post},
    state::{AppState, POST_COLUMNS},
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
    posts: Vec<Post>,
    flash: Option<String>,
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
         (SELECT COUNT(*) FROM posts p WHERE p.feed_id = f.id) AS post_count,
         EXISTS(SELECT 1 FROM posts p WHERE p.feed_id = f.id
                AND p.published_at >= datetime('now', '-48 hours')) AS has_new_post
         FROM feeds f WHERE f.id = ? AND f.status = 'active'";

pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FeedQuery>,
) -> AppResult<Html<String>> {
    let feed = sqlx::query_as::<_, Feed>(FEED_SQL)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;

    let posts = sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLUMNS} FROM posts p JOIN feeds f ON f.id = p.feed_id
         WHERE p.feed_id = ? ORDER BY p.published_at DESC, p.id DESC LIMIT 50"
    )))
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let flash = match query.msg.as_deref() {
        Some("added") => Some("Feed added. Posts will appear as they are fetched.".to_string()),
        _ => None,
    };

    Ok(Html(FeedTemplate { feed, posts, flash }.render()?))
}

pub async fn read(State(state): State<AppState>, Path(id): Path<i64>) -> AppResult<Html<String>> {
    let post = sqlx::query_as::<_, Post>(sqlx::AssertSqlSafe(format!(
        "SELECT {POST_COLUMNS} FROM posts p JOIN feeds f ON f.id = p.feed_id
         WHERE p.id = ? AND f.status = 'active'"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let youtube_id = post.youtube_id().map(str::to_string);
    let cover = post.cover_image();
    Ok(Html(
        ReaderTemplate {
            post,
            cover,
            youtube_id,
        }
        .render()?,
    ))
}
