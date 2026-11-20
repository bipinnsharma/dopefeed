#!/usr/bin/env python3
"""Reconcile local posts with topfeed.xyz, the reference directory.

topfeed is the source of truth for which posts a feed has: for every local feed
that maps to a topfeed feed, missing posts are inserted as link stubs, titles
and dates are refreshed, and local-only rows are deleted. Feed rows (names,
URLs, categories) are never touched, and topfeed feeds with no entries are
skipped so a stale reference cannot empty a working feed.

Idempotent. Dry run by default; pass --apply before `scripts/snapshot.sh`.

    python3 scripts/topfeed_sync.py [--apply] [--db data/dopefeed.db]
"""

import argparse
import collections
import datetime as dt
import hashlib
import json
import os
import re
import sqlite3
import sys
import time
import urllib.parse
import urllib.request

API = "https://topfeed.xyz/api/feeds"
UA = "dopefeed/0.1 (feed directory snapshot)"
SITE_DOMAIN = re.compile(r"^[a-z0-9-]+(\.[a-z0-9-]+)*$")
# An `--apply` followed by `--verify` refetches everything; reuse responses for
# an hour. /data is gitignored.
CACHE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "topfeed-cache")
CACHE_TTL = 3600


def cached(url):
    path = os.path.join(CACHE, hashlib.sha256(url.encode()).hexdigest() + ".json")
    try:
        if time.time() - os.path.getmtime(path) < CACHE_TTL:
            with open(path) as handle:
                return json.load(handle)
    except (OSError, ValueError):
        pass
    return None


def remember(url, data):
    path = os.path.join(CACHE, hashlib.sha256(url.encode()).hexdigest() + ".json")
    try:
        os.makedirs(CACHE, exist_ok=True)
        with open(path + ".tmp", "w") as handle:
            json.dump(data, handle)
        os.replace(path + ".tmp", path)
    except OSError as error:  # a cache miss is always recoverable
        print(f"cache write failed: {error}", file=sys.stderr)


def get(url):
    hit = cached(url)
    if hit is not None:
        return hit
    for attempt in range(5):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=45) as res:
                data = json.loads(res.read())
            remember(url, data)
            time.sleep(0.25)  # topfeed resets connections on bursts
            return data
        except urllib.error.HTTPError as error:
            if error.code == 429:
                wait = int(error.headers.get("Retry-After") or 0) or 30
                print(f"rate limited, waiting {wait}s", file=sys.stderr)
                time.sleep(wait)
                continue
            if error.code == 404 or attempt == 4:
                raise
        except Exception as error:  # noqa: BLE001 - retried below
            if attempt == 4:
                raise
            print(f"retrying {url}: {error}", file=sys.stderr)
        time.sleep(2 * (attempt + 1))


def all_feeds():
    feeds, page = [], 1
    while True:
        data = get(f"{API}?page={page}&pageSize=100")
        feeds += data["items"]
        if page * data["pageSize"] >= data["total"]:
            return feeds
        page += 1


def entries(slug):
    items, page, page_size, total = [], 1, None, None
    while True:
        try:
            data = get(f"{API}/{urllib.parse.quote(slug)}?page={page}&pageSize=100")
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None  # topfeed lists it, but the entries endpoint is gone
            raise
        block = data.get("entries", {})
        current_size, current_total, current_items = (
            block.get("pageSize"), block.get("total"), block.get("items")
        )
        if (not isinstance(current_size, int) or current_size <= 0
                or not isinstance(current_total, int) or current_total < 0
                or not isinstance(current_items, list)):
            print(f"  skipping {slug}: invalid entries pagination metadata", file=sys.stderr)
            return None
        if page_size is None:
            page_size, total = current_size, current_total
        if current_size != page_size or current_total != total or len(current_items) > page_size:
            print(f"  skipping {slug}: inconsistent entries pagination metadata", file=sys.stderr)
            return None
        expected = min(page_size, total - len(items))
        if len(current_items) != expected:
            print(f"  skipping {slug}: incomplete entries page {page} ({len(current_items)}/{expected})", file=sys.stderr)
            return None
        items.extend(current_items)
        if len(items) == total:
            return items
        page += 1


FEED_FILE = re.compile(r"(feed|rss|atom|index|posts|blog)\.?(xml|atom|rss|json)?", re.I)
TRACKING = {
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "ref",
    "ref_src",
}


def url_key(url):
    """Host and path with the feed filename stripped, so /rss, /feed.xml and
    /posts.atom for one blog compare equal. YouTube needs its ids: every video
    shares the /watch path."""
    if not url:
        return ("", "")
    value = url.strip()
    if value.startswith("yt:video:"):
        return ("youtube.com", "watch:" + value.removeprefix("yt:video:"))
    parsed = urllib.parse.urlparse(value)
    host = (parsed.hostname or "").lower().removeprefix("www.")
    if host in ("youtube.com", "m.youtube.com", "youtu.be"):
        query = urllib.parse.parse_qs(parsed.query)
        if "channel_id" in query:
            return ("youtube.com", "channel:" + query["channel_id"][0])
        if "v" in query:
            return ("youtube.com", "watch:" + query["v"][0])
        segments = [part for part in parsed.path.split("/") if part]
        if len(segments) > 1 and segments[0] in ("shorts", "live", "embed"):
            return ("youtube.com", "watch:" + segments[1])
        if host == "youtu.be" and segments:
            return ("youtube.com", "watch:" + segments[0])
        return ("youtube.com", "/".join(segments))
    segments = [part for part in parsed.path.split("/") if part]
    while segments and FEED_FILE.fullmatch(segments[-1]):
        segments.pop()
    if segments and segments[0].lower() == "feed":
        segments.pop(0)
    # Permalinks like ?p=42 differ only by query, so keep it (minus tracking).
    query = "&".join(
        sorted(
            f"{key}={value}"
            for key, value in urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
            if key.lower() not in TRACKING
        )
    )
    return (host, "/".join(segments) + ("?" + query if query else ""))


def sqlite_time(ms):
    return dt.datetime.fromtimestamp(ms / 1000, dt.timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


def map_feeds(feeds, rows):
    """Attach each local feed to one topfeed feed. Exact feed URL first, then
    the same site with the same path, then a unique host."""
    by_key = collections.defaultdict(list)
    by_host = collections.defaultdict(list)
    for feed in feeds:
        by_key[url_key(feed["url"])].append(feed)
        host = (urllib.parse.urlparse(feed.get("websiteUrl") or feed["url"]).hostname or "")
        by_host[host.lower().removeprefix("www.")].append(feed)

    local_hosts = collections.defaultdict(list)
    for row in rows:
        for host in local_hosts_for(row):
            local_hosts[host].append(row)

    used, mapping = set(), {}
    for row in rows:
        key = url_key(row["url"])
        options = [feed for feed in by_key.get(key, []) if feed["slug"] not in used]
        if not options:
            for host in local_hosts_for(row):
                candidates = [feed for feed in by_host.get(host, []) if feed["slug"] not in used]
                # Only a one-to-one exact host match is safe as a fallback.
                if len(local_hosts.get(host, [])) == 1 and len(candidates) == 1:
                    options = candidates
                    break
        if len(options) > 1:
            print(f"  skipping local feed {row['id']}: ambiguous reference match", file=sys.stderr)
            continue
        if options:
            feed = options[0]
            if feed["totalEntriesCount"] > 0:
                used.add(feed["slug"])
                mapping[row["id"]] = feed
    return mapping


def local_hosts_for(row):
    hosts = set()
    for value in (row["site_url"], row["domain"], row["url"]):
        host = (urllib.parse.urlparse(value or "").hostname or "").lower().removeprefix("www.")
        if host and SITE_DOMAIN.match(host):
            hosts.add(host)
    return hosts


def entry_url(item, feed):
    """Some topfeed feeds (pj-onori-s-blog) return site-relative URLs."""
    url = (item.get("url") or "").strip()
    if url.startswith("//"):
        return "https:" + url
    if url and not url.startswith(("http://", "https://")):
        url = urllib.parse.urljoin(feed.get("websiteUrl") or feed["url"], url)
    return url if url.startswith(("http://", "https://")) else ""


def sync_feed(db, row, feed, apply):
    remote = entries(feed["slug"])
    if remote is None:
        print(f"  skipping {feed['slug']}: entries endpoint returned 404", file=sys.stderr)
        return None
    if not remote:
        print(f"  skipping {feed['slug']}: reference has no entries", file=sys.stderr)
        return None
    keep, claimed, inserted, updated = set(), set(), 0, 0

    def donor_for(key, exclude=None):
        candidates = [
            pid for pid in row["by_key"].get(key, [])
            if pid != exclude and pid not in claimed
        ]
        candidates.sort(key=lambda pid: not row["filled"].get(pid))
        return candidates[0] if candidates else None

    def update(post_id, item):
        title = (item.get("title") or "").strip()
        published = item.get("publishedAt")
        if not title or not published:
            return 0
        if apply:
            db.execute(
                "UPDATE posts SET title = ?, published_at = ?, "
                "summary = CASE WHEN summary = '' THEN ? ELSE summary END WHERE id = ?",
                (title, sqlite_time(published), (item.get("contentSnippet") or "").strip(), post_id),
            )
        return 1

    for item in remote:
        url = entry_url(item, feed)
        if not url:
            continue
        # topfeed counts one post per exact URL string: http/https, fragment,
        # tracking-query and trailing-slash variants are separate entries.
        exact = row["by_url"].get(url)
        if exact is not None and exact not in claimed:
            # An empty stub (topfeed's URL spelling) must not shadow the row
            # that carries the body (the feed's spelling).
            if not row["filled"].get(exact):
                donor = donor_for(url_key(url), exclude=exact)
                if donor is not None and row["filled"].get(donor):
                    claimed.add(donor)
                    claimed.add(exact)
                    keep.add(donor)
                    updated += update(donor, item)
                    continue
            claimed.add(exact)
            keep.add(exact)
            updated += update(exact, item)
            continue
        donor = donor_for(url_key(url))
        if donor is not None:
            claimed.add(donor)
            keep.add(donor)
            updated += update(donor, item)
            continue
        keep.add(("new", url))
        if apply:
            before = db.total_changes
            # topfeed's entry id dodges `UNIQUE(feed_id, guid)` when the same
            # article already exists under another URL (lukew.com).
            db.execute(
                "INSERT OR IGNORE INTO posts (feed_id, guid, url, title, content_html, summary,"
                " thumb_url, published_at) VALUES (?, ?, ?, ?, '', ?, '', ?)",
                (
                    row["id"],
                    item.get("_id") or url,
                    url,
                    (item.get("title") or "").strip() or url,
                    (item.get("contentSnippet") or "").strip(),
                    sqlite_time(item.get("publishedAt")) if item.get("publishedAt") else sqlite_time(0),
                ),
            )
            inserted += db.total_changes - before
        else:
            inserted += 1
    stale = [post_id for post_id in row["post_ids"] if post_id not in keep]
    if apply and stale:
        db.executemany("DELETE FROM posts WHERE id = ?", ((post_id,) for post_id in stale))
    return len(keep), inserted, updated, len(stale)


def self_check():
    """The mapping decides what gets deleted, so pin the URL rules."""
    global entries
    assert url_key("https://www.example.com/feed.xml") == url_key("http://example.com/rss")
    assert url_key("https://sive.rs/feed.xml") == url_key("https://sive.rs/feed")
    assert url_key("https://blog.samaltman.com/posts.atom") == ("blog.samaltman.com", "")
    assert url_key(
        "https://www.youtube.com/feeds/videos.xml?channel_id=UCabc"
    ) == ("youtube.com", "channel:UCabc")
    assert url_key("https://www.youtube.com/watch?v=abc") == ("youtube.com", "watch:abc")
    assert url_key("https://youtu.be/abc") == url_key("https://www.youtube.com/shorts/abc")
    assert url_key("yt:video:abc") == ("youtube.com", "watch:abc")
    assert url_key("https://every.to/source-code/feed") != url_key(
        "https://every.to/superorganizers/feed"
    )
    assert url_key("https://shud.in/thoughts/a#atom") == ("shud.in", "thoughts/a")
    assert url_key("https://example.com/?p=1") != url_key("https://example.com/?p=2")
    assert url_key("https://example.com/post?utm_source=rss") == ("example.com", "post")
    assert not map_feeds(
        [{"slug": "b", "title": "B", "url": "https://b.co.uk/feed", "websiteUrl": "https://b.co.uk", "totalEntriesCount": 1}],
        [{"id": 1, "title": "A", "domain": "a.co.uk", "url": "https://a.co.uk/feed", "site_url": "https://a.co.uk"}],
    ), "unrelated .co.uk hosts must not map"
    ambiguous = [
        {"slug": slug, "title": slug, "url": f"https://api.example/{slug}", "websiteUrl": "https://example.com", "totalEntriesCount": 1}
        for slug in ("one", "two")
    ]
    assert not map_feeds(ambiguous, [{"id": 1, "title": "Local", "domain": "example.com", "url": "https://example.com/feed", "site_url": "https://example.com"}])

    global get
    saved_get = get
    pages = iter([
        {"entries": {"pageSize": 2, "total": 3, "items": [{"url": "/1"}, {"url": "/2"}]}},
        {"entries": {"pageSize": 2, "total": 3, "items": []}},
    ])
    get = lambda _url: next(pages)
    try:
        assert entries("partial") is None, "partial reference must be rejected"
    finally:
        get = saved_get

    feed = {"url": "https://pjonori.blog/rss.xml", "websiteUrl": "https://pjonori.blog"}
    assert entry_url({"url": "/posts/x"}, feed) == "https://pjonori.blog/posts/x"
    assert entry_url({"url": "//cdn.example/x"}, feed) == "https://cdn.example/x"
    assert entry_url({"url": "https://other.example/y"}, feed) == "https://other.example/y"

    # topfeed's URL spelling must not shadow a row that has the body, and a
    # second entry for the same article must add a row, not delete content.
    import sqlite3 as sqlite

    saved_entries = entries
    test = sqlite.connect(":memory:")
    test.executescript(
        "CREATE TABLE posts (id INTEGER PRIMARY KEY, feed_id INTEGER, url TEXT, guid TEXT,"
        " title TEXT, content_html TEXT, summary TEXT, thumb_url TEXT, published_at TEXT)"
    )
    test.executemany(
        "INSERT INTO posts VALUES (?, 9, ?, ?, '', ?, '', '', '2020-01-01')",
        [
            (1, "https://maggie.appleton/x/", "https://maggie.appleton/x/", ""),
            (2, "https://maggie.appleton/x", "https://maggie.appleton/x", "<p>body</p>"),
            (3, "https://maggie.appleton/y", "https://maggie.appleton/y", ""),
        ],
    )
    key = url_key("https://maggie.appleton/x")
    row = {
        "id": 9,
        "post_ids": [1, 2, 3],
        "by_url": {
            "https://maggie.appleton/x/": 1,
            "https://maggie.appleton/x": 2,
            "https://maggie.appleton/y": 3,
        },
        "by_key": {key: [1, 2], url_key("https://maggie.appleton/y"): [3]},
        "filled": {1: False, 2: True, 3: False},
    }
    entry = {"title": "X", "publishedAt": 1700000000000, "contentSnippet": ""}
    entries = lambda slug: [
        {**entry, "url": "https://maggie.appleton/x/", "_id": "a"},
        {**entry, "url": "http://maggie.appleton/x", "_id": "b"},
    ]
    try:
        kept, inserted, _, deleted = sync_feed(test, row, {"slug": "test"}, True)
    finally:
        entries = saved_entries
    assert (kept, inserted, deleted) == (2, 1, 2), (kept, inserted, deleted)
    assert test.execute("SELECT count(*) FROM posts").fetchone()[0] == 2
    assert test.execute("SELECT content_html FROM posts WHERE id = 2").fetchone()[0] == "<p>body</p>"

    entries = lambda _slug: []
    try:
        before = test.execute("SELECT count(*) FROM posts").fetchone()[0]
        assert sync_feed(test, row, {"slug": "empty"}, True) is None
        assert test.execute("SELECT count(*) FROM posts").fetchone()[0] == before
    finally:
        entries = saved_entries

    import shutil
    import tempfile

    global CACHE
    saved, CACHE = CACHE, tempfile.mkdtemp()
    scratch = CACHE
    try:
        remember("https://cache.test/x", {"ok": 1})
        assert cached("https://cache.test/x") == {"ok": 1}
        assert cached("https://cache.test/y") is None
    finally:
        CACHE = saved
        shutil.rmtree(scratch)
    feeds = [
        {"slug": "blog", "title": "Blog", "url": "https://example.com/rss", "websiteUrl": "https://example.com", "totalEntriesCount": 3},
        {"slug": "empty", "title": "Empty", "url": "https://empty.example/feed", "websiteUrl": "https://empty.example", "totalEntriesCount": 0},
    ]
    rows = [
        {"id": 1, "title": "Blog", "domain": "example.com", "url": "https://example.com/feed.xml", "site_url": "https://example.com"},
        {"id": 2, "title": "Empty", "domain": "empty.example", "url": "https://empty.example/rss", "site_url": "https://empty.example"},
    ]
    mapping = map_feeds(feeds, rows)
    assert mapping[1]["slug"] == "blog"
    assert 2 not in mapping, "a reference feed with no entries must be skipped"
    print("self-check ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--apply", action="store_true", help="write changes (default: dry run)")
    parser.add_argument("--db", default="data/dopefeed.db")
    parser.add_argument("--self-check", action="store_true", help="check URL matching and exit")
    parser.add_argument("--verify", action="store_true", help="compare post counts and exit")
    args = parser.parse_args()

    if args.self_check:
        self_check()
        return

    db = sqlite3.connect(args.db)
    db.row_factory = sqlite3.Row
    rows = db.execute(
        "SELECT id, title, domain, url, site_url FROM feeds ORDER BY id"
    ).fetchall()
    local = []
    for row in rows:
        posts = db.execute(
            "SELECT id, url, guid, content_html <> '' AS filled FROM posts WHERE feed_id = ?",
            (row["id"],),
        ).fetchall()
        by_url, by_key = {}, collections.defaultdict(list)
        filled = {}
        for post in posts:
            pid = post["id"]
            filled[pid] = bool(post["filled"])
            if post["url"]:
                by_url[post["url"].strip()] = pid
            for key in {url_key(value) for value in (post["url"], post["guid"]) if value}:
                by_key[key].append(pid)
        local.append({
            **dict(row),
            "post_ids": [post["id"] for post in posts],
            "by_url": by_url,
            "by_key": by_key,
            "filled": filled,
        })

    mapping = map_feeds(all_feeds(), local)
    print(f"matched {len(mapping)}/{len(local)} local feeds")
    if args.verify:
        bad = []
        for row in local:
            feed = mapping.get(row["id"])
            if not feed:
                continue
            count = len(row["post_ids"])
            if count != feed["totalEntriesCount"]:
                bad.append((row["id"], row["title"], count, feed["totalEntriesCount"]))
        for feed_id, title, count, expected in bad:
            print(f"  {feed_id:3} {title[:30]:30} local={count:5} topfeed={expected:5}")
        print(f"{len(bad)} mismatched feeds")
        sys.exit(1 if bad else 0)
    if args.apply:
        print("applying")
    total_inserted = total_deleted = 0
    for row in local:
        feed = mapping.get(row["id"])
        if not feed:
            continue
        before = len(row["post_ids"])
        result = sync_feed(db, row, feed, args.apply)
        if result is None:
            continue
        kept, inserted, updated, stale = result
        total_inserted += inserted
        total_deleted += stale
        if inserted or stale:
            print(
                f"  {row['id']:3} {row['title'][:30]:30} {before:5} -> {kept:5}"
                f"  +{inserted:4} -{stale:4}  ({feed['slug']})"
            )
    if args.apply:
        db.commit()
        print(f"inserted {total_inserted}, deleted {total_deleted}")
    else:
        print(f"would insert {total_inserted}, delete {total_deleted} (dry run)")


if __name__ == "__main__":
    main()
