use crate::{
    config::Config, models::Post, routes, state::AppState, worker::fetch, worker::fetch::DueFeed,
    worker::ssrf,
};
use reqwest::{
    Client,
    header::{CONTENT_TYPE, ORIGIN},
    redirect::Policy,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{net::SocketAddr, str::FromStr};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const RSS: &[u8] = include_bytes!("../fixtures/rss2.xml");
const ATOM: &[u8] = include_bytes!("../fixtures/atom.xml");
const YOUTUBE: &[u8] = include_bytes!("../fixtures/youtube.xml");
const MALFORMED: &[u8] = include_bytes!("../fixtures/malformed.xml");

async fn test_pool() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

fn test_config(base_url: &str) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        base_url: base_url.parse().unwrap(),
        database_url: "sqlite::memory:".into(),
        fetch_concurrency: 2,
    }
}

async fn test_state() -> AppState {
    AppState::new(test_pool().await, test_config("http://127.0.0.1:3000")).unwrap()
}

/// Binds first so the submitted Origin matches the app's BASE_URL.
async fn spawn_app() -> (String, AppState) {
    let pool = test_pool().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let state = AppState::new(pool, test_config(&base)).unwrap();
    let app = routes::router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (base, state)
}

async fn add_feed(pool: &SqlitePool, url: &str, status: &str) -> i64 {
    sqlx::query(
        "INSERT INTO feeds (url, site_url, domain, title, category, status)
         VALUES (?, 'https://example.com', 'example.com', 'Fixture Feed', 'tech', ?)",
    )
    .bind(url)
    .bind(status)
    .execute(pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn add_post(pool: &SqlitePool, feed_id: i64, guid: &str, title: &str, published_at: &str) {
    sqlx::query(
        "INSERT INTO posts (feed_id, guid, url, title, content_html, summary, published_at)
         VALUES (?, ?, 'https://example.com/post', ?, '<p>body</p>', 'summary', ?)",
    )
    .bind(feed_id)
    .bind(guid)
    .bind(title)
    .bind(published_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn post_count(pool: &SqlitePool, feed_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ---------- M2: fetch worker ----------

#[test]
fn fixtures_parse() {
    let rss = feed_rs::parser::parse(RSS).unwrap();
    assert_eq!(rss.entries.len(), 3);
    let atom = feed_rs::parser::parse(ATOM).unwrap();
    assert_eq!(atom.entries.len(), 2);
    let youtube = feed_rs::parser::parse(YOUTUBE).unwrap();
    assert_eq!(youtube.entries[0].id, "yt:video:dQw4w9WgXcQ");
    assert!(feed_rs::parser::parse(MALFORMED).is_err());
}

#[tokio::test]
async fn ingest_dedups_and_sanitizes() {
    let state = test_state().await;
    let id = add_feed(&state.pool, "https://example.com/feed.xml", "active").await;
    let feed = feed_rs::parser::parse(RSS).unwrap();

    let first = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    let second = fetch::ingest(&state.pool, id, &feed, "https://example.com")
        .await
        .unwrap();
    assert_eq!(first, 3);
    assert_eq!(second, 0);
    assert_eq!(post_count(&state.pool, id).await, 3);

    let content: String = sqlx::query_scalar(
        "SELECT content_html FROM posts WHERE feed_id = ? AND guid = 'fixture-1'",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(content.contains("<strong>world</strong>"));
    for forbidden in ["<script", "<iframe", "<style", "<form"] {
        assert!(!content.contains(forbidden), "sanitizer kept {forbidden}");
    }

    let summary: String =
        sqlx::query_scalar("SELECT summary FROM posts WHERE feed_id = ? AND guid = 'fixture-1'")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(summary.chars().count() <= 300);
    assert!(summary.contains("short summary"));
}

#[tokio::test]
async fn refresh_handles_304_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: Some("\"abc\"".into()),
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        post_count: 0,
    };
    fetch::refresh(&state, &due).await.unwrap();
    assert_eq!(post_count(&state.pool, id).await, 0);
    let (errors, fetched): (i64, Option<String>) =
        sqlx::query_as("SELECT error_count, last_fetched_at FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(errors, 0);
    assert!(fetched.is_some());
}

#[tokio::test]
async fn refresh_success_adapts_interval() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_raw(RSS, "application/rss+xml"),
        )
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        post_count: 0,
    };
    fetch::refresh(&state, &due).await.unwrap();

    assert_eq!(post_count(&state.pool, id).await, 3);
    let (interval, errors, etag): (i64, i64, Option<String>) =
        sqlx::query_as("SELECT fetch_interval_secs, error_count, etag FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(interval, 1800, ">=3 new posts halves the interval");
    assert_eq!(errors, 0);
    assert_eq!(etag.as_deref(), Some("\"v1\""));
}

#[tokio::test]
async fn refresh_failure_backs_off() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/feed.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/feed.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        post_count: 0,
    };
    fetch::refresh(&state, &due).await.unwrap();

    let (errors, last_error, delay): (i64, Option<String>, i64) = sqlx::query_as(
        "SELECT error_count, last_error, strftime('%s', next_fetch_at) - strftime('%s','now')
         FROM feeds WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(errors, 1);
    assert_eq!(last_error.as_deref(), Some("HTTP 500"));
    assert!(
        (7100..=7300).contains(&delay),
        "3600 * 2^1 backoff, got {delay}"
    );
}

#[tokio::test]
async fn refresh_rejects_oversized_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big.xml"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![b'a'; ssrf::MAX_BODY + 1], "text/xml"),
        )
        .mount(&server)
        .await;
    let state = test_state().await;
    let id = add_feed(&state.pool, &format!("{}/big.xml", server.uri()), "active").await;
    let due = DueFeed {
        id,
        url: format!("{}/big.xml", server.uri()),
        site_url: "https://example.com".into(),
        etag: None,
        last_modified: None,
        fetch_interval_secs: 3600,
        error_count: 0,
        domain: "127.0.0.1".into(),
        post_count: 0,
    };
    fetch::refresh(&state, &due).await.unwrap();
    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM feeds WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert!(last_error.unwrap().contains("too large"));
}

// ---------- SSRF ----------

#[tokio::test]
async fn ssrf_blocks_private_and_local_targets() {
    for blocked in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.1.1",
        "0.0.0.0",
        "0.1.2.3",
        "100.64.0.1",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
    ] {
        let ip = blocked.parse().unwrap();
        assert!(ssrf::is_blocked(ip), "{blocked} should be blocked");
    }
    for allowed in ["93.184.216.34", "2606:4700::1111"] {
        let ip = allowed.parse().unwrap();
        assert!(!ssrf::is_blocked(ip), "{allowed} should be allowed");
    }
    assert!(
        ssrf::check_url(&"http://10.0.0.1/feed".parse().unwrap())
            .await
            .is_err()
    );
    assert!(
        ssrf::check_url(&"ftp://example.com/feed".parse().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resolver_rejects_blocked_addresses() {
    use reqwest::dns::Resolve;

    let name = "0.0.0.0".parse().unwrap();
    assert!(ssrf::SafeResolver.resolve(name).await.is_err());
}

// ---------- M4: timeline ----------

#[tokio::test]
async fn timeline_validates_and_filters() {
    let (base, state) = spawn_app().await;
    let feed_a = add_feed(&state.pool, "https://a.example/feed", "active").await;
    let feed_b = add_feed(&state.pool, "https://b.example/feed", "active").await;
    let rejected = add_feed(&state.pool, "https://c.example/feed", "rejected").await;
    for index in 0..55 {
        add_post(
            &state.pool,
            feed_a,
            &format!("a-{index}"),
            &format!("A post {index}"),
            &format!("2025-01-{:02} 10:00:00", index % 28 + 1),
        )
        .await;
    }
    add_post(&state.pool, feed_b, "b-1", "B post", "2025-02-01 10:00:00").await;
    add_post(
        &state.pool,
        rejected,
        "c-1",
        "Rejected post",
        "2025-02-01 10:00:00",
    )
    .await;

    let client = Client::new();
    let page_one = client
        .get(format!("{base}/api/timeline?feeds={feed_a}"))
        .send()
        .await
        .unwrap();
    assert_eq!(page_one.status(), 200);
    let body = page_one.text().await.unwrap();
    assert_eq!(body.matches("data-post-id=").count(), 50);
    assert!(body.contains("A post"));
    assert!(!body.contains("B post"));
    assert!(!body.contains("Rejected post"));

    let page_two = client
        .get(format!("{base}/api/timeline?feeds={feed_a}&page=2"))
        .send()
        .await
        .unwrap();
    let body = page_two.text().await.unwrap();
    assert_eq!(body.matches("data-post-id=").count(), 5);

    for bad in ["feeds=abc", "feeds=1,abc", "feeds="] {
        let response = client
            .get(format!("{base}/api/timeline?{bad}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{bad} should be rejected");
    }
    let many = vec!["1"; 51].join(",");
    let response = client
        .get(format!("{base}/api/timeline?feeds={many}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn timeline_rejects_injection() {
    let (base, state) = spawn_app().await;
    let feed = add_feed(&state.pool, "https://a.example/feed", "active").await;
    add_post(&state.pool, feed, "a-1", "A post", "2025-01-01 10:00:00").await;

    let client = Client::new();
    for bad in ["1;DROP%20TABLE%20posts", "1%20OR%201%3D1", "1)--"] {
        let response = client
            .get(format!("{base}/api/timeline?feeds={bad}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{bad} should be rejected");
    }
    assert_eq!(post_count(&state.pool, feed).await, 1);
}

#[tokio::test]
async fn directory_query_values_are_bound() {
    let (base, state) = spawn_app().await;
    add_feed(&state.pool, "https://one.example/feed", "active").await;
    let client = Client::new();

    let injected = client
        .get(format!("{base}/feeds?q=%27%20OR%201%3D1--"))
        .send()
        .await
        .unwrap();
    assert_eq!(injected.status(), 200);
    assert!(!injected.text().await.unwrap().contains("Fixture Feed"));

    // Invalid categories never reach SQL; they fall back to no category filter.
    let category = client
        .get(format!("{base}/feeds?category=%27%20OR%201%3D1--"))
        .send()
        .await
        .unwrap();
    assert_eq!(category.status(), 200);
    assert!(category.text().await.unwrap().contains("Fixture Feed"));
}

#[test]
fn sqlite_time_round_trips_as_space_format() {
    use chrono::{TimeZone, Utc};

    let time = Utc.with_ymd_and_hms(2026, 11, 12, 10, 30, 0).unwrap();
    assert_eq!(fetch::sqlite_time(time), "2026-11-12 10:30:00");

    let from_feed = chrono::DateTime::parse_from_rfc3339("2026-11-12T10:30:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(fetch::sqlite_time(from_feed), "2026-11-12 10:30:00");

    let parsed =
        chrono::NaiveDateTime::parse_from_str("2026-11-12 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
    assert_eq!(parsed.and_utc(), time);
}

#[tokio::test]
async fn no_cookies_are_set() {
    let (base, state) = spawn_app().await;
    add_feed(&state.pool, "https://one.example/feed", "active").await;
    let client = Client::new();
    for path in ["/", "/my", "/about"] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(
            response.headers().get("set-cookie").is_none(),
            "{path} set a cookie"
        );
    }
    let home = client
        .get(format!("{base}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(home.contains("Fixture Feed"));
}

// ---------- M5: submission ----------

async fn submit(client: &Client, base: &str, url: &str) -> reqwest::Response {
    client
        .post(format!("{base}/submit"))
        .header(ORIGIN, base)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(format!("url={url}&category=tech"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn submit_html_alternate_inserts_and_fetches() {
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml"))
        .mount(&feed_server)
        .await;
    let page = format!(
        r#"<html><head><link rel="alternate" type="application/rss+xml" href="{}/feed.xml"></head><body>site</body></html>"#,
        feed_server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&feed_server)
        .await;

    let (base, state) = spawn_app().await;
    let client = Client::builder().redirect(Policy::none()).build().unwrap();
    let url = format!("{}/page", feed_server.uri());
    let response = submit(&client, &base, &url).await;
    assert_eq!(response.status(), 303);
    let location = response
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.starts_with("/f/"));
    assert!(location.ends_with("?msg=added"));

    let (status, title): (String, String) =
        sqlx::query_as("SELECT status, title FROM feeds ORDER BY id DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(status, "active");
    assert_eq!(title, "Fixture Feed");
    let feed_id: i64 = sqlx::query_scalar("SELECT id FROM feeds ORDER BY id DESC LIMIT 1")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(post_count(&state.pool, feed_id).await, 3);

    let duplicate = submit(&client, &base, &url).await;
    assert_eq!(duplicate.status(), 400);
    assert!(duplicate.text().await.unwrap().contains("already"));
}

#[tokio::test]
async fn submit_rejects_missing_origin() {
    let (base, _state) = spawn_app().await;
    let client = Client::new();
    let response = client
        .post(format!("{base}/submit"))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("url=http://10.0.0.1/feed&category=tech")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn submit_rate_limits_after_five() {
    let (base, _state) = spawn_app().await;
    let client = Client::new();
    for attempt in 1..=5 {
        let response = submit(&client, &base, "http://10.0.0.1/feed").await;
        assert_eq!(response.status(), 400, "attempt {attempt}");
    }
    let sixth = submit(&client, &base, "http://10.0.0.1/feed").await;
    assert_eq!(sixth.status(), 429);
}

#[tokio::test]
async fn submit_blocks_redirect_to_private() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://10.0.0.1/feed"))
        .mount(&server)
        .await;
    let (base, state) = spawn_app().await;
    let client = Client::new();
    let response = submit(&client, &base, &format!("{}/redirect", server.uri())).await;
    assert_eq!(response.status(), 400);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn youtube_channel_id_is_extracted() {
    let html =
        r#"{"channelId":"UCaaaaaaaaaaaaaaaaaaaaaa","externalId":"UCbbbbbbbbbbbbbbbbbbbbbb"}"#;
    assert_eq!(
        crate::worker::discover::extract_channel_id(html).as_deref(),
        Some("UCaaaaaaaaaaaaaaaaaaaaaa")
    );
    let html = r#"<link href="https://www.youtube.com/channel/UCcccccccccccccccccccccc">"#;
    assert_eq!(
        crate::worker::discover::extract_channel_id(html).as_deref(),
        Some("UCcccccccccccccccccccccc")
    );
}

#[tokio::test]
async fn seed_placeholder_keeps_real_url_and_error() {
    let state = test_state().await;
    crate::seed::insert_placeholder(
        &state,
        "https://unreachable.invalid",
        "Dead Feed",
        "tech",
        "dns failed",
    )
    .await
    .unwrap();
    crate::seed::insert_placeholder(
        &state,
        "https://unreachable.invalid",
        "Dead Feed",
        "tech",
        "dns failed",
    )
    .await
    .unwrap();

    let (url, error, status): (String, Option<String>, String) =
        sqlx::query_as("SELECT url, last_error, status FROM feeds")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(url, "https://unreachable.invalid/");
    assert_eq!(error.as_deref(), Some("dns failed"));
    assert_eq!(status, "active");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn youtube_guid_extracts_embed_id() {
    let post = Post {
        id: 1,
        feed_id: 1,
        feed_title: "t".into(),
        feed_domain: "d".into(),
        guid: "yt:video:dQw4w9WgXcQ".into(),
        url: "u".into(),
        title: "t".into(),
        author: None,
        summary: String::new(),
        content_html: String::new(),
        published_label: "Jan 01, 2025".into(),
    };
    assert_eq!(post.youtube_id(), Some("dQw4w9WgXcQ"));
}

#[test]
fn feed_parser_drops_empty_text_elements() {
    let xml = br#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/">
<title>t</title><id>f</id><updated>2024-01-01T00:00:00Z</updated>
<entry><id>e</id><title>e</title><updated>2024-01-01T00:00:00Z</updated>
<media:group><media:description></media:description></media:group></entry>
</feed>"#;
    assert!(feed_rs::parser::parse(xml.as_slice()).is_err());
    let feed = crate::worker::discover::parse_feed(xml).unwrap();
    assert_eq!(feed.entries.len(), 1);
}
