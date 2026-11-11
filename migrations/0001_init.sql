CREATE TABLE feeds (
  id INTEGER PRIMARY KEY,
  url TEXT NOT NULL UNIQUE,
  site_url TEXT NOT NULL,
  domain TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  category TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  etag TEXT, last_modified TEXT,
  last_fetched_at TEXT,
  next_fetch_at TEXT NOT NULL DEFAULT (datetime('now')),
  fetch_interval_secs INTEGER NOT NULL DEFAULT 3600,
  error_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE posts (
  id INTEGER PRIMARY KEY,
  feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  guid TEXT NOT NULL,
  url TEXT NOT NULL,
  title TEXT NOT NULL,
  author TEXT,
  content_html TEXT NOT NULL,
  summary TEXT NOT NULL DEFAULT '',
  published_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(feed_id, guid)
);
CREATE INDEX idx_posts_feed_pub ON posts(feed_id, published_at DESC);
CREATE INDEX idx_posts_pub ON posts(published_at DESC);
CREATE INDEX idx_feeds_next ON feeds(status, next_fetch_at);
CREATE INDEX idx_feeds_cat ON feeds(category);
