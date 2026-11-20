-- Preserve IDs while preventing SQLite from reusing a deleted maximum rowid.
CREATE TABLE feeds_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
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
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  avatar_url TEXT,
  banner_url TEXT,
  images_fetched_at TEXT,
  favicon_fetched_at TEXT
);
INSERT INTO feeds_new SELECT * FROM feeds;

-- Older syncs could leave posts behind after their feed row was removed.
-- Keep those posts and restore referential integrity without guessing a source.
INSERT INTO feeds_new (id, url, site_url, domain, title, category, status)
SELECT DISTINCT p.feed_id,
       'https://archived-feed-' || p.feed_id || '.invalid/',
       'https://archived-feed-' || p.feed_id || '.invalid/',
       'archived-feed-' || p.feed_id || '.invalid',
       'Archived feed ' || p.feed_id,
       'tech',
       'rejected'
FROM posts p
LEFT JOIN feeds f ON f.id = p.feed_id
WHERE f.id IS NULL;

CREATE TABLE posts_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  feed_id INTEGER NOT NULL REFERENCES feeds_new(id) ON DELETE CASCADE,
  guid TEXT NOT NULL,
  url TEXT NOT NULL,
  title TEXT NOT NULL,
  author TEXT,
  content_html TEXT NOT NULL,
  summary TEXT NOT NULL DEFAULT '',
  published_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  thumb_url TEXT,
  content_hash TEXT,
  UNIQUE(feed_id, guid)
);
INSERT INTO posts_new SELECT * FROM posts;

-- Rebuild the child first: dropping the old parent must not cascade to it.
DROP TABLE posts;
DROP TABLE feeds;
ALTER TABLE feeds_new RENAME TO feeds;
ALTER TABLE posts_new RENAME TO posts;

CREATE INDEX idx_posts_feed_pub ON posts(feed_id, published_at DESC);
CREATE INDEX idx_posts_pub ON posts(published_at DESC);
CREATE INDEX idx_feeds_next ON feeds(status, next_fetch_at);
CREATE INDEX idx_feeds_cat ON feeds(category);
