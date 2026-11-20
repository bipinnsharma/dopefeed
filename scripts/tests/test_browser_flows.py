#!/usr/bin/env python3
"""Rendered-browser regressions for directory, local state, and reader flows."""
import asyncio
import os
from pathlib import Path
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.request import urlopen

from playwright.async_api import async_playwright, expect

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target" / "debug" / "dopefeed"


def seed_database(path):
    db = sqlite3.connect(path)
    feeds = []
    for index in range(26):
        cursor = db.execute(
            "INSERT INTO feeds (url, site_url, domain, title, category, status) "
            "VALUES (?, ?, ?, ?, 'ai', 'active')",
            (
                f"https://source{index}.example/feed",
                f"https://source{index}.example",
                f"source{index}.example",
                f"Target Feed {index:02}",
            ),
        )
        feed_id = cursor.lastrowid
        feeds.append(feed_id)
        db.execute(
            "INSERT INTO posts (feed_id, guid, url, title, content_html, summary, published_at) "
            "VALUES (?, 'post', ?, ?, '<p>Read me</p>', 'Summary', '2025-01-01 00:00:00')",
            (feed_id, f"https://source{index}.example/article", f"Article {index:02}"),
        )
    cursor = db.execute(
        "INSERT INTO posts (feed_id, guid, url, title, content_html, summary, published_at, thumb_url) "
        "VALUES (?, 'xss', 'https://source0.example/xss', 'XSS regression', '<p>body</p>', '', '2025-02-01', ?)" ,
        (feeds[0], "https://example.invalid/';window.auditPwned=true;//"),
    )
    db.commit()
    xss_post = cursor.lastrowid
    db.close()
    return feeds, xss_post


async def wait_ready(url, process):
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError("dopefeed exited before becoming ready")
        try:
            with urlopen(url + "/healthz", timeout=1) as response:
                if response.status == 200:
                    return
        except Exception:
            time.sleep(0.1)
    raise TimeoutError("dopefeed did not become ready")


async def run():
    if not BINARY.is_file():
        raise RuntimeError(f"missing {BINARY}; build dopefeed before running this test")
    with tempfile.TemporaryDirectory(prefix="dopefeed-browser-") as directory:
        db_path = Path(directory) / "browser.db"
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        env = os.environ.copy()
        env.update(
            BIND_ADDR=f"127.0.0.1:{port}",
            BASE_URL=base,
            DATABASE_URL=f"sqlite://{db_path}?mode=rwc",
            WORKER="0",
            RUST_LOG="error",
            ADMIN_KEY="",
            READ_ONLY="false",
        )
        process = subprocess.Popen([str(BINARY)], cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            await wait_ready(base, process)
            feed_ids, xss_post = seed_database(db_path)
            async with async_playwright() as playwright:
                browser = await playwright.chromium.launch(headless=True)
                context = await browser.new_context()
                page = await context.new_page()

                # Filter -> search -> sort -> load more preserves all filters.
                requests = []
                page.on("request", lambda req: requests.append(req.url) if "/feeds?" in req.url else None)
                await page.goto(base)
                async with page.expect_response(lambda response: "/feeds?" in response.url and "category=ai" in response.url):
                    await page.get_by_role("link", name="AI", exact=True).click()
                await expect(page.locator("#feed-count")).to_contain_text("26")
                async with page.expect_response(lambda response: "/feeds?" in response.url and "q=Target" in response.url):
                    await page.locator("#search-input").fill("Target")
                    await page.locator("#search-input").press("Enter")
                await expect(page.locator("#feed-count")).to_contain_text("26")
                await page.locator("select[name=sort]").select_option("popular")
                await expect(page).to_have_url(re.compile(r"category=ai.*q=Target.*sort=popular"))
                async with page.expect_response(lambda response: "page=2" in response.url and "more=1" in response.url):
                    await page.locator("#feed-more button").click()
                await expect(page.locator("#feed-grid button[data-feed-id]")).to_have_count(26)
                assert any("category=ai" in url and "q=Target" in url and "sort=popular" in url and "page=2" in url for url in requests)

                # The newly inserted card is delegated, persists, and supplies /my.
                button = page.locator("#feed-grid button[data-feed-id]").last
                subscribed_id = await button.get_attribute("data-feed-id")
                await button.click()
                await expect(button).to_have_attribute("aria-pressed", "true")
                assert str(subscribed_id) in await page.evaluate("localStorage.getItem('dopefeed.subs')")
                await page.reload()
                await expect(page.locator(f'button[data-feed-id="{subscribed_id}"]')).to_have_attribute("aria-pressed", "true")
                await page.goto(base + "/my")
                row = page.locator(".post-row").first
                await expect(row).to_be_visible()
                await row.click()
                await expect(page).to_have_url(re.compile(r"/read/\d+"))
                await page.go_back()
                await expect(page.locator(".post-row.is-read")).to_have_count(1)

                # A full stored list gets an explicit limit message and cannot grow.
                cap_page = await context.new_page()
                await cap_page.add_init_script("localStorage.setItem('dopefeed.subs', JSON.stringify(Array.from({length:50}, (_,i)=>1000+i)))")
                await cap_page.goto(base)
                before = await cap_page.evaluate("localStorage.getItem('dopefeed.subs')")
                await cap_page.locator("button[data-feed-id]").first.click()
                await expect(cap_page.get_by_role("status").filter(has_text="up to 50 feeds")).to_be_visible()
                assert await cap_page.evaluate("localStorage.getItem('dopefeed.subs')") == before

                # Existing over-limit state remains intact and explains recovery instead of blanking /my.
                recovery = await context.new_page()
                await recovery.add_init_script("localStorage.setItem('dopefeed.subs', JSON.stringify(Array.from({length:51}, (_,i)=>5000+i)))")
                timeline_requests = []
                recovery.on("request", lambda req: timeline_requests.append(req.url) if "/api/timeline" in req.url else None)
                await recovery.goto(base + "/my")
                await expect(recovery.locator("#timeline")).to_contain_text("51 saved subscriptions")
                await expect(recovery.get_by_role("link", name="Browse the directory")).to_be_visible()
                assert not timeline_requests
                assert await recovery.evaluate("JSON.parse(localStorage.getItem('dopefeed.subs')).length") == 51

                # The reader fallback handler stays fixed even with a hostile feed-controlled URL.
                await context.route("https://example.invalid/**", lambda route: route.abort())
                await page.add_init_script("window.auditPwned = false")
                await page.goto(f"{base}/read/{xss_post}")
                await page.wait_for_function("document.querySelector('.reader-cover img').complete && document.querySelector('.reader-cover img').naturalWidth === 0")
                assert await page.evaluate("window.auditPwned") is False
                assert "this.dataset.fallback" in await page.locator(".reader-cover img").get_attribute("onerror")

                await browser.close()
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    asyncio.run(run())
