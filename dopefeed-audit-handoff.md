# Dopefeed audit fixes — cross-session handoff

## Start here

Project: `/Users/bipin/Documents/dopefeed`

User objective: fix the gaps identified in a whole-project audit, using parallel subagents where practical. This document supplies the audit context and execution brief for a new session.

Read this document, `agent.md`, `README.md`, and the current code before changing anything. Reproduce or verify each finding; this is an audit snapshot, not a substitute for inspecting the current implementation.

**Priority:** reader code injection, ineffective admin throttling, and destructive reference synchronization first. Then durable production behavior, safe publishing, and browser flows.

If the new session is in Plan mode, switch to an implementation agent before editing project files or launching implementation subagents.

## Current execution status

- The audit and delegation brief were completed. Four implementation agents from the prior session returned `Free usage limit reached` without reporting completed work.
- This session verified main checkout HEAD is still `6cf6de38062ebe546c2e7637b5908bc080d0033d` (`master`). The four `fix/...` worktrees are clean at that same HEAD; no fixes are present there.
- Prior unrelated worktrees under `/Users/bipin/.local/share/opencode/worktree/36b79f/` are on other commits; do not integrate them without inspecting their diff and ancestry.
- Finding 1 is fixed in the main checkout. A full HTTP-render regression confirms the malicious fallback URL remains escaped data and the handler reads only `dataset.fallback`; targeted and full Rust tests pass.
- Finding 2 is now fixed in the main checkout: lockout is checked before credentials are verified; failures are recorded only after incorrect credential attempts; admin keys require at least 12 printable ASCII characters. README and fixtures were updated. `cargo test admin_` passes (5 tests).
- Finding 3 is now fixed in the main checkout: reference pages are validated against consistent pagination totals before returning; empty/incomplete pages skip reconciliation; unsafe last-two-label host matching is removed; only one-to-one exact-host fallback is allowed and ambiguous matches are skipped. Python self-check passes including partial, empty, `.co.uk`, and ambiguity cases.
- Finding 4 is now fixed in the main checkout: `READ_ONLY` defaults false, invalid values fail config loading, Vercel sets it true, public/admin mutation routes reject writes before discovery/database side effects, and GET `/submit` explains why the form is unavailable. Added a regression test covering public submit, admin create/delete, GET UI, and no rows added/deleted.
- Finding 5 is now fixed in the main checkout: migration 0007 rebuilds both tables with AUTOINCREMENT, preserves existing IDs/data/indexes/constraints, and retains FK cascade behavior. Populated pre-0007 migration regression passes.
- Finding 6 is now fixed in the main checkout: the snapshot script rejects staged/modified work and unrelated untracked files before refresh, permits and retains only untracked generated `static/assets/`, and stages only `data/snapshot.txt` plus `static/assets/`. Temporary-repository regression mocks every publication command and confirms staged/source/unrelated changes are not swept in.
- Finding 7 is now fixed in the main checkout: each compressed DB upload uses a timestamp/PID/content-hash filename, GitHub upload metadata is validated before the pointer changes, the pointer update is atomic, and previous assets are never deleted. Temporary tests cover failed upload and invalid metadata preserving the old pointer and successful pointer update without deleting older assets.
- Findings 8–10 implementation is complete pending browser verification: `static/app.js` delegates subscription clicks and repaints subscription/read state after htmx swaps, history restoration, and cross-tab storage changes; directory category/query/sort values are synchronized; the client blocks a 51st subscription and shows a recovery message for preexisting over-limit lists. Server boundary test checks 50 IDs accepted and 51 rejected. Rendered Playwright cases are added but not yet executed.
- Finding 11 is fixed: ingest updates both existing-GUID/hash and URL maps after insert/update. Added one-feed duplicate-response coverage for repeated GUID, repeated URL with a different GUID, later valid entry, and repeat ingest.
- Finding 12 is fixed in code: ammonia resolves sanitized URL attributes against each article URL; a version marker in content hashes forces reprocessing of fetched entries; a bounded, rerunnable `repair-body-urls` command repairs historical bodies in 500-row batches, including posts from rejected feeds. Added tests for root/path/protocol-relative/absolute/fragment/unsafe links and 501-row batch traversal. Do not run this command on the real local database during development.
- Finding 13 read-only inspection was recomputed from `data/dopefeed.db` on 2026-11-20: 26,008 posts (5,497 with non-empty bodies; 843 video, 27 with bodies; among non-video 5,470 bodies, 8,995 summary-only, 10,700 with neither body nor summary). `thumb_url=''` marks 20,038 bodyless rows, but the schema cannot prove all are Topfeed-imported. 173 feeds, 12 currently with error_count>0, four with >=3 consecutive errors; all 173 fetched within the last day. Highest persistent errors: Making Software (17, 429), Huozhi (11, discovery failure), Magur (10, 403), Reid Hoffman (3, 403). Recommendation: manually verify stale/blocked endpoints; don't add scraping or collector changes without the directory-first vs reader-first decision.
- User-requested Charlie Deets correction: seed source is `discover:https://charliedeets.com/writing/`. The live Next.js writing archive serializes records in its Flight payload instead of server-rendering anchors; added a narrow parser for that archive, which emits canonical `/posts/{slug}` links, dates, excludes hidden records, and deduplicates repeated category listings. Migration 0008 retains canonical `/posts/` archive entries, removes `/work/` and legacy `/writings/` entries, and normalizes trailing slashes. User approved keeping only writing archive entries.
- Local restart/data update was explicitly authorized and backed up. Backup before migrations: `/private/var/folders/_p/rcs58klx5rsfnn5y2pv429rc0000gn/T/opencode/dopefeed-restart.uoHcea/dopefeed.db` (`integrity_check=ok`). A second safety backup of the temporary post-refresh duplicate state is at `/private/var/folders/_p/rcs58klx5rsfnn5y2pv429rc0000gn/T/opencode/dopefeed-charlie-before-reapply.Xhb3T4/current.db`. Migration 7 discovered 15 pre-existing posts attached to missing feed ID 22; it now preserves them under a rejected placeholder without guessing source identity. Restored the pre-migration backup and replayed corrected migrations 7–8. Ran only targeted `refresh-feed 234` (new CLI) with app worker disabled. Live Charlie feed ID 234 has 18 archive items: 7 in 2026, 8 in 2025, and 1 each in 2022, 2021, and 2018; no work/legacy paths or duplicate trailing-slash entries. DB integrity and FK checks pass. Localhost `/healthz` returns 200. Server remains launched with `ADMIN_KEY=` and `WORKER=0`; admin is disabled and background refresh is off. `.env` unchanged.
- Finding 14 implementation is complete pending CI: `.github/workflows/ci.yml` and `scripts/check.sh` run Rust format/clippy/tests (including SSRF feature), sync self-check, publication safety tests, and a Python Playwright browser flow. `snapshot.sh` runs the same checks before refresh or upload. Browser runner/Chromium are installed by CI; unavailable locally, so rendered browser tests and integrated `scripts/check.sh` have not run here.
- Validation after latest changes: standard Rust suite passes (68 tests, including orphan preservation, Charlie migration/path normalization, and Next payload parsing), Clippy and `cargo fmt --check` pass. Previous checks remain: `ssrf-test-exception` suite (66 tests before the new migration coverage), Python sync self-check, `node --check static/app.js`, temporary snapshot safety tests, Python browser-test compilation, shell syntax checks, and `git diff --check`. Browser test is still pending CI.
- Finding 7 checks: `sh -n scripts/snapshot.sh`, `sh -n scripts/tests/test_snapshot_safety.sh`, and `sh scripts/tests/test_snapshot_safety.sh` pass. No real upload/publication was performed. Latest focused checks: `node --check static/app.js`, `cargo test timeline_validates_and_filters`, and `python3 scripts/topfeed_sync.py --self-check` pass.
- Current uncommitted implementation files also include `migrations/0008_charlie_deets_writing_feed.sql`, the Charlie seed update in `src/seed.rs`, the archive parser in `src/worker/discover.rs`, targeted refresh command in `src/{main.rs,worker/mod.rs}`, its tests, and README command docs. Existing untracked assets remain user data; preserve them and do not stage or delete them.
- Latest visual polish: wide `.app-main` uses `--surface-base` (`#fffaf0` light, `#171512` dark), and inactive category chips use `--color-accent-strong` on hover; selected chips retain their active treatment.
- Continue sequentially unless delegation is explicitly available. Review each fix and test it before marking it done. Update this status after each meaningful finding/checkpoint so another session can resume safely.

### Resume instructions

1. Read this handoff and inspect `git status`/`git diff`; do not discard the listed working changes or pre-existing untracked assets.
2. Findings 1–12 and 14 have code/tests, but findings 8–10 and 14 still need actual CI/browser execution. No browser runner is installed locally. Finding 13 is read-only complete; ask the user to choose directory-first labeling vs reader-first acquisition before any collection scope is added.
3. After CI/browser results, fix any failures and update finding-by-finding status. Do not add an application Node/npm build.
4. No publication, push, deploy, real database migration/repair, or production upload without separate user authorization.
- No successful publication, push, or deployment was performed by this work.
- Main checkout was on `master`; its observed HEAD was `6cf6de3` (`Snapshot 2026-11-20`). Recheck HEAD: an external snapshot process may advance it.
- Existing untracked files included `src/.DS_Store` and many `static/assets/posts/*` images. Preserve them. The images may be current worker output.

Existing worktrees:

| Assignment | Directory |
|---|---|
| Reader and ingestion | `/Users/bipin/.local/share/opencode/worktree/c38e28/fix/reader-ingest` |
| Authentication and read-only mode | `/Users/bipin/.local/share/opencode/worktree/c38e28/fix/admin-ro` |
| Synchronization and stable IDs | `/Users/bipin/.local/share/opencode/worktree/c38e28/fix/sync-ids` |
| Browser flows | `/Users/bipin/.local/share/opencode/worktree/c38e28/fix/browser` |

The worktrees were initially detached at the observed HEAD. Agent prompts requested creation of matching `fix/...` branches, but this was not confirmed. Inspect before creating branches or deleting anything. If delegation is still unavailable, implement the same assignments sequentially rather than claiming agents completed them.

## Architecture and constraints

- Rust, axum, sqlx/SQLite, Askama templates, vanilla JavaScript, vendored htmx.
- No accounts; subscriptions and read history are numeric IDs in localStorage.
- Feed fetching/discovery and image caching run in a worker.
- Vercel runs with `WORKER=0` against `/tmp/dopefeed.db`, seeded from a snapshot. Instance writes are temporary.
- `scripts/snapshot.sh` refreshes the local database, reconciles against Topfeed, optimizes/verifies images, uploads the database to a GitHub release, then commits and pushes.
- `scripts/topfeed_sync.py` deliberately treats Topfeed as the reference for mapped feed membership. Preserve that intended policy for verified complete responses while fixing unsafe deletion cases.
- Keep changes small and consistent with existing code. Do not add accounts, scraping infrastructure, broad refactors, or an application Node/npm build.
- Use existing dependencies and helpers. Add focused regressions for security, data writes, and broken flows.
- Preserve IDs, article bodies, existing URLs, constraints, and foreign-key behavior during migrations.
- Test with in-memory or disposable databases. Never run migrations, refresh, synchronization, or repair commands against the real `data/dopefeed.db` during implementation.
- Never run the real publishing script, upload assets, push, or deploy during testing. Mock external publication commands.
- Do not print secret values. Do not stage unrelated files with `git add -A`.

## Audit evidence and scope

Read-only local database inspection on 2026-11-20 found:

- 174 feeds and 26,094 posts.
- 17 feeds with recorded fetch errors; examples: Making Software (17 failures, HTTP 429), Huozhi (11, discovery failure), Magur (10, HTTP 403).
- 20,601 posts with empty bodies overall.
- Excluding known YouTube/video URLs and GUIDs: 18,791 of 24,257 posts had empty bodies, approximately 77%.
- 426 articles contained root-relative body links or images.
- All 2,494 distinct database-referenced local image paths existed. This did not verify every derived cover variant or Blob object.

Isolated checks reproduced the inline image-handler JavaScript injection, empty-reference deletion, unrelated `.co.uk` mapping, and SQLite ID reuse. The JavaScript check exercised the generated handler pattern, not the full browser/submission pipeline.

The full Cargo suite, Docker build, live Vercel configuration, Blob configuration, and real-device accessibility were not verified during the audit. Existing Rust tests cover many server paths, but the repository had no browser suite or `.github` CI workflow.

## Findings: numbered reference

Line numbers below refer to the audited version; search for the symbol if they have moved.

### 1. Reader image fallback permits JavaScript injection — fixed in progress

References: `templates/reader.html:29–30`, `src/models.rs:175–189`, `src/assets.rs:184–216`.

The template interpolates a feed-controlled fallback URL into an inline JavaScript string. A URL such as `https://example.com/';globalThis.auditExecuted=true;//` survives URL normalization. HTML-escaped apostrophes decode before the inline handler executes. If caching fails and the image errors, injected code executes in the site's origin.

Smallest fix: put the fallback URL in an HTML-escaped data attribute and use a fixed handler reading `this.dataset.fallback`. Disable the handler before applying the fallback to prevent loops. Check sibling templates for the same pattern; constant-only fallback strings are not this injection bug.

Regression: malicious URL remains data and cannot execute code; normal fallback works. Preserve HTML sanitization.

Status: changed `templates/reader.html` to store the fallback in an escaped `data-fallback` attribute and use a fixed handler that disables itself before fallback. Added an HTTP rendering regression in `src/tests.rs`; targeted and full suites pass. The existing placeholder helper test also remains green.

### 2. Admin throttle checks passwords during lockout — fixed in progress

References: `src/routes/admin.rs:197–265` (`auth`, `authorized`, `over_limit`), `src/config.rs:16–20` (`valid_admin_key`).

`auth()` checks the password before consulting the failure limit. After the threshold, wrong guesses receive 429, but every guess is still checked and a correct guess is accepted. Configuration also forces exactly six alphanumeric characters.

Fix: enforce lockout before verifying another credential, preserve unauthenticated Basic-auth negotiation, and accept strong longer secrets under a documented validation rule. Preserve constant-time comparison. Do not blindly trust client-supplied forwarded-IP headers.

Regressions: failed attempts trigger lockout; even correct credentials remain blocked during that window; access resumes after expiry; long secrets work; no key still disables admin. Update existing short-key fixtures and tests deliberately.

Status: `auth` checks the per-IP lockout before password verification and records only failed credential attempts. Config validation now requires 12+ printable ASCII characters; README and fixtures are updated. Added HTTP coverage for lockout and expiry. Full suite and clippy pass. Forwarded-IP headers remain untrusted (peer `ConnectInfo` is still the limiter key).

### 3. Reference synchronization can erase valid bodies — must fix

References: `scripts/topfeed_sync.py:96–109` (`entries`), `165–220` (`site`, `map_feeds`), `242–321` (`sync_feed`).

An empty entries list is treated as authoritative and deletes all local posts. Pagination can return a partial list after an empty page while the advertised total still indicates more entries. Separately, taking the final two hostname labels maps unrelated `a.co.uk` and `b.co.uk` feeds when their paths match. Host fallback can also select among ambiguous remote candidates.

Fix: validate completeness and consistency before destructive reconciliation; skip empty/incomplete/ambiguous reference results with an explanation. Remove unsafe hostname reduction; use exact normalized matching and genuinely unambiguous fallbacks. Keep intentional stale-row removal for verified nonempty complete references.

Regressions: empty and partial results delete nothing; unrelated `.co.uk` sites do not match; ambiguous matches are skipped; URL-variant donor matching retains article bodies; dry-run and repeated apply remain correct.

Status: `entries()` now validates pagination fields, cross-page consistency, and exact expected item counts; empty/incomplete results return `None` and `sync_feed()` skips them before writes. Removed registrable-domain suffix matching; exact URL-key match remains, with only unique exact-host fallback. Multiple candidates are skipped. Updated README and self-check; Python self-check and the integrated Rust suite/clippy pass.

### 4. Production mutations are temporary — fixed in progress

References: `Dockerfile.vercel:31–33`, `docker-entrypoint.sh:3–6`, `src/routes/submit.rs:69–117,175–237`, `src/routes/admin.rs:70–138`, `templates/submit.html`.

Public submissions and admin create/delete mutate only one ephemeral Vercel database. Changes disappear when an instance is replaced and are not shared with other instances. The form promises addition without explaining this.

Fix for the current snapshot architecture: add an explicit `READ_ONLY` configuration setting, default off for self-hosting, and enable it in the Vercel image. Reject public/admin mutations before network or database side effects. Explain availability on GET `/submit`; keep browsing and inspection available. Do not silently replace the storage architecture.

Regressions: every mutation is blocked in read-only mode, no network discovery or write starts, and writable mode retains existing behavior.

Status: added `READ_ONLY` (default false; only boolean syntax accepted) and documented it; set `READ_ONLY=true` in `Dockerfile.vercel`. Public submit, admin create/delete, and the shared `insert_resolved` path reject mutations. GET `/submit` displays an explanation and omits the form. Regression `read_only_mode_blocks_mutations_before_discovery_or_writes` passes; writable behavior remains covered by existing tests. Full suite (63 tests) and clippy pass. Docker build and live Vercel behavior not checked.

### 5. SQLite ID reuse corrupts browser identity — must fix

References: `migrations/0001_init.sql:2,19`, `static/app.js:201–235`, admin deletion and Topfeed deletion paths.

Plain `INTEGER PRIMARY KEY` can reuse the highest deleted ID. Browser subscriptions and read flags persist that ID, so a replacement feed may inherit subscriptions or a new article may appear read.

Fix: a new migration to non-reused IDs, such as SQLite AUTOINCREMENT, preserving all existing IDs and schema behavior. Inspect every migration to reconstruct the actual current table definitions.

Important: changing `PRAGMA foreign_keys=OFF` inside an already-open SQLite transaction does not disable enforcement. Do not assume a table rebuild is safe just because it contains that pragma. Account for sqlx's migration transaction and foreign-key cascades; test with populated parent and child tables before accepting the migration.

Regressions: migration retains IDs, rows, indexes, and uniqueness; deleting highest IDs does not allow reuse; foreign-key checks pass; feed deletion still cascades posts. Previously deleted historical IDs cannot necessarily be reconstructed; document that limitation.

Status: migration `0007_autoincrement_ids.sql` rebuilds `feeds` and `posts` with `INTEGER PRIMARY KEY AUTOINCREMENT`, copies all current schema columns/rows, and recreates the four indexes. It avoids toggling `foreign_keys` inside the migration transaction. The populated-schema regression starts at migrations 0001–0006, checks preserved IDs/body and uniqueness, deletes maximum IDs, verifies both new IDs exceed the deleted maxima, confirms cascades and an empty `PRAGMA foreign_key_check`. `cargo test` passes (64 tests). Previously deleted historical high-water IDs cannot be recovered; AUTOINCREMENT starts from surviving maximum IDs.

### 6. Snapshot publication stages unrelated work — must fix

Reference: `scripts/snapshot.sh:51–57`.

`git add -A` followed by commit/push includes unfinished source changes and unrelated untracked files, including the observed `src/.DS_Store`.

Fix: validate checkout suitability before refresh/upload side effects, use a clean publishing checkout when necessary, and stage explicitly allowed outputs. Account for legitimate generated untracked images rather than indiscriminately deleting them. Preserve existing staged/unstaged user work.

Regressions in a temporary repository: unrelated files cannot be committed, pre-staged source changes are not swept in, and unsuitable checkouts fail before external effects.

Status: preflight now rejects staged/modified paths and all untracked files outside `static/assets/` before acquiring the publish lock or running cargo/network steps. Untracked generated images are retained and allowed. Staging is explicit (`data/snapshot.txt`, `static/assets`) rather than `git add -A`. `sh scripts/tests/test_snapshot_safety.sh` passes using temporary repos and mocked publication commands; it checks early refusal, generated/pre-existing asset inclusion, and unrelated-file exclusion. No real publication was run.

### 7. Snapshot assets are mutable and replacement is unsafe — must fix

References: `scripts/snapshot.sh:42–49`, `Dockerfile.vercel:24–26`, `data/snapshot.txt`.

The script deletes and replaces the same `db-latest/dopefeed.db.xz` asset. A timestamp query changes cache behavior, not asset identity. Rebuilding an old commit can download a newer database; failed replacement can leave no asset.

Fix: immutable uniquely named assets, uploaded and validated before updating the pointer. Retain prior referenced assets for rollback. Keep upload/pointer/commit ordering recoverable on failure.

Regressions with mocked upload commands: failed upload leaves the old pointer and asset usable; a previous pointer continues identifying identical bytes after another publication; old code/image sets can use their matching snapshot.

Status: the publisher creates `dopefeed-<UTC timestamp>-<pid>-<sha256 prefix>.db.xz`, validates GitHub's returned name, byte size, upload state, and download URL, then atomically replaces `data/snapshot.txt`. It never deletes release assets. The temporary-repo test has a prior pointer/asset, confirms upload failure and invalid metadata preserve the pointer, and successful publication changes the pointer with no DELETE request. Shell syntax and `sh scripts/tests/test_snapshot_safety.sh` pass. No live GitHub asset was touched.

### 8. Dynamic cards lack handlers and posts lack read styling — implemented; browser verification pending

Reference: `static/app.js:204–235,452–463`, htmx partial templates.

Subscriptions and read-state painting initialize once. New cards from filtering/load-more have no click handler. `/my` loads its rows after the initial styling pass. Back/forward cache restoration can retain stale styling.

Fix: delegated subscription clicks; idempotent state painting after htmx swaps and on history restoration, reading current storage as needed. Avoid duplicate listeners and preserve `aria-pressed`.

Regressions: filter/load-more then subscribe, reload persistence, `/my` read styling, and reader → browser Back.

Status: `static/app.js` now uses a single delegated subscription listener, paints all current buttons and post rows from storage idempotently, and refreshes after `htmx:afterSwap`, `pageshow`, and relevant `storage` events. Click toggles repaint all buttons, including duplicates. JS parses with `node --check`; browser interaction regressions remain unverified because no browser runner is installed.

### 9. Category/search/sort submit stale state — implemented; browser verification pending

References: `templates/directory.html:5–11,18–45`, `static/app.js:246–271`.

Category chips change only the grid. Hidden category/query values in search/sort forms retain the original page values. Selecting AI then searching can send the original empty category; sorting restores stale filters.

Fix: synchronize the active filter values across request sources. Preserve query, category, and sort together and keep displayed controls consistent with successful results.

Regression: choose AI → search → change sort → load more; every request and result retains the intended values.

Status: category-chip clicks now update the hidden category in both the htmx search form and GET sort form; search input changes update the sort form's hidden query. Existing pagination URLs are built from server-returned request filters. JS syntax passes; end-to-end request sequence still needs browser verification.

### 10. Subscription 51 breaks `/my` — implemented; browser verification pending

References: `static/app.js:213–215,459`, `src/routes/my.rs:35–39`.

The UI accepts unlimited subscriptions while the API rejects more than 50 IDs. The failed request leaves the timeline without a useful explanation.

Fix: enforce the same limit with a clear user message. Existing over-limit lists need an understandable recovery path; do not silently delete saved subscriptions or silently hide the excess.

Regressions: 50 works, the 51st addition is explained, unsubscription still works, and a previously stored over-limit list does not leave an unexplained blank timeline.

Status: client refuses a 51st new subscription with a clear toast; unsubscription remains allowed. `/my` detects stored lists over 50, preserves them, and shows a count plus directory recovery link instead of issuing a request that returns 400. Server regression now verifies a 50-ID request returns 200 and 51 remains rejected. JS syntax passes; client click/over-limit UI still requires browser verification.

### 11. Duplicates within one response abort ingestion — should fix

References: `src/worker/fetch.rs:377–387,429–499` (`ingest`).

The existing-row lookup is built before processing. New inserts update `by_url` but not `existing`, so a repeated new GUID or a second entry mapped to that new GUID attempts another INSERT and hits uniqueness, leaving a partial import.

Fix: update both in-memory maps consistently after insert and preserve existing stub-repair semantics. The audited `MAX_ENTRIES` is 1000, despite older comments/docs saying 500; do not change that limit as an unrelated fix.

Regressions: same GUID twice, same URL with different GUIDs, duplicate followed by a valid entry, and repeat ingestion. Existing stub IDs and bodies must survive.

Status: after each insert or update, `ingest()` now updates the in-memory GUID/hash and URL maps. The regression includes repeated GUID, repeated URL/different GUID, a later valid entry, and repeat ingestion; targeted and full Rust tests pass. Existing `ingest_fills_a_stub_by_url` remains green.

### 12. Relative body links resolve against Dopefeed — should fix

References: `src/worker/fetch.rs:442–449`, `templates/reader.html:53`, `src/models.rs:118–133`.

The first-image extractor resolves URLs, but stored article HTML does not. Root-relative article links and image paths resolve against Dopefeed. This was observed in 426 stored articles.

Fix: resolve relative HTML URLs against the article URL using existing URL/sanitization facilities. Preserve unsafe-scheme filtering. Ensure the content-hash optimization does not skip corrected processing forever. Plan a bounded repair for existing bodies, including historical posts no longer present in a feed; do not run it on the real database during development.

Regressions: root-relative, path-relative, protocol-relative, absolute, fragment, and unsafe URLs; sanitization still removes executable markup.

Status: `clean_body()` applies Ammonia's `RewriteWithBase` using the article URL while retaining default URL-scheme sanitization. `BODY_URL_VERSION` changes content hashes so fetched existing posts are not skipped. Added `repair-body-urls` CLI, processing 500 rows at a time across all stored nonempty posts; rerunnable and covers rejected-feed/history rows. Regression repairs 501 rows over multiple batches, asserts each URL class and unsafe markup removal, then verifies a second run changes zero rows. Full Rust suite passes (66 tests). Run only against a backed-up/local publishing DB when actually deploying; not run in this work.

### 13. Reader coverage and source failures need a product decision — investigate

References: `scripts/topfeed_sync.py:303–312`, `src/worker/fetch.rs:345–367`, `src/routes/static_pages.rs:70–72`, `templates/_post_row.html`, `templates/reader.html`.

Most non-video rows are link stubs. Some feeds have recurring refresh errors. `/healthz` checks database access only, so it does not reveal stale snapshots or stalled collection.

First recompute current figures and separate video entries, full bodies, source summaries, and imported stubs where the schema permits. Recommend repairs for recurring high-impact failures. Ask the user to choose directory-first labeling versus a reader-first content acquisition scope before adding collection features.

Propose lightweight visibility for ingestion freshness/failures without breaking the meaning of liveness. Do not add scraping services or infrastructure without that product decision.

Status: read-only inspection recomputed from the local DB as summarized in execution status. Errors are concentrated in 4 feeds: repeated 429/403 and one recurring discovery failure; all feeds fetched in the last day. Schema distinguishes body, summary-only, and empty rows, but cannot reliably identify every imported stub. No collector/health behavior changed. Product decision still required: directory-first labeling vs reader-first content acquisition.

### 14. Publication lacks verification gates — implemented; CI verification pending

References: `src/tests.rs`, `scripts/snapshot.sh:21–24`, `scripts/smoke.sh`, `Dockerfile.vercel:11`; no repository `.github` workflow was found.

Publishing builds without running the server tests. HTTP assertions do not execute the browser interactions in findings 8–10.

Fix: repository CI and pre-publication gates for Rust formatting/lint/tests, Python sync regressions, and a focused browser test for filtering → subscription → timeline → reading. Use tooling compatible with the project's no-application-Node-build constraint. Document how checks run and keep them independent of production secrets/data.

Regression: failing checks prevent publication. Browser tests must actually interact with rendered pages, not merely assert source strings.

Status: added CI workflow, shared `scripts/check.sh`, and a snapshot preflight invocation of that check before refresh/upload. Added a Playwright Python end-to-end test that interacts with rendered category/search/sort/pagination, dynamic subscribe/reload, `/my`, reader/back read styling, 50/51 subscription behavior, over-limit recovery, and malicious image fallback. Python runner is pinned in `scripts/tests/requirements.txt`; no application Node/npm build. CI installs Chromium. Local Rust/shell/Python syntax checks pass; actual browser test and whole check script remain unverified locally because Playwright/Chromium aren't installed.

## Delegation and file ownership

Use isolated worktrees. Agents 1–4 can run in parallel. Agent 5 starts after their changes are integrated. Agent 6 can investigate in parallel but stops for the product decision.

### Agent 1 — reader and ingestion: findings 1, 11, 12

- Own `templates/reader.html`, `src/worker/fetch.rs`, and focused tests/repair code.
- Inspect sibling templates for injection, but do not make unrelated fallback changes.
- Avoid `static/app.js` (Agent 4 owns it); a fixed inline handler reading a data attribute suffices for finding 1.
- Coordinate any migration with Agent 3 before choosing a number. Prefer a solution that does not require a competing migration if equally correct.
- Run formatting, lint, and relevant Rust tests. Explain how existing stored bodies receive the fix.

### Agent 2 — auth and read-only mode: findings 2, 4

- Own `src/config.rs`, admin/submit handlers, admin/submit templates, and configuration documentation.
- Update all `Config` construction sites and auth fixtures.
- Give Agent 5 the exact environment setting for the Vercel image; Agent 5 owns that deployment edit.
- Keep read-only enforcement before external side effects and preserve local writable defaults.
- Test lockout expiry without real multi-minute sleeps, using controlled timestamps or an appropriate test clock.

### Agent 3 — synchronization and identities: findings 3, 5

- Own `scripts/topfeed_sync.py`, new identity migration(s), and migration tests.
- Coordinate migration numbering with Agent 1; never independently pick the same prefix.
- Validate migration using pre-migration populated schemas, not only a fresh empty database.
- Extend `--self-check` or focused Python tests for empty, partial, ambiguous, and URL-variant cases.

### Agent 4 — browser flows: findings 8, 9, 10

- Own `static/app.js`, `templates/directory.html`, and directly related filter/subscription partials.
- Keep vanilla JavaScript and existing conventions; avoid unrelated toast/animation rewrites.
- Preserve keyboard behavior, accessible state, and reduced-motion support.
- Verify in a real browser when available and give Agent 5 the exact regression scenarios/tooling.

### Agent 5 — publication and checks: findings 6, 7, 14

- Start after Agents 1–4 are integrated.
- Own `scripts/snapshot.sh`, publication tests, `scripts/smoke.sh` if needed, `Dockerfile.vercel`, CI, browser regression integration, and deployment documentation.
- Enable Agent 2's read-only setting in Vercel.
- Mock GitHub uploads and git pushes; use temporary repositories for failure-path tests.
- Retain old snapshot assets; do not perform a production cleanup.
- Account for compiler/test fixture availability in CI and the browser runner's setup.

### Agent 6 — content quality: finding 13

- Initial work is read-only. Do not repair the actual database or change product scope.
- Recompute coverage/failure counts, identify repair priorities, and return a short recommendation.
- Ask the user for the directory-first versus reader-first decision before implementing collection changes.

## Coordinator responsibilities

1. Inspect main checkout and all worktrees. Preserve user changes and confirm a consistent base before integrating.
2. Pass each agent its finding descriptions and constraints from this file. Do not rely on another session's conversational context.
3. Review proposed security and migration changes personally; passing tests alone is insufficient.
4. Resolve shared `src/tests.rs`, configuration fixture, and README conflicts by preserving every relevant regression. Do not discard an agent's tests to make a merge easy.
5. Use explicit file staging for any local commits. Do not push, publish, or deploy without a separate user instruction.
6. Integrate in a safe order: migration/schema work, backend fixes, browser fixes, then publication/checks. Adjust for actual dependencies.
7. Recheck whether the live checkout's HEAD advanced during parallel work before integration.

Each implementation agent returns:

- Findings confirmed, corrected, or not reproduced.
- Files changed and why.
- Tests run with actual outcomes.
- Remaining limitations or user decisions.
- Local commit ID, if committed.

## Final validation and completion report

Run against the integrated code, using isolated test resources:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features ssrf-test-exception
python3 scripts/topfeed_sync.py --self-check
```

Also run the added publication failure-path tests and actual browser regression suite. Verify the populated-schema migration, `PRAGMA foreign_key_check`, and read-only production configuration. Inspect a Docker build if available; report if unavailable rather than claiming it passed. Do not accidentally test with production environment variables or assets that trigger uploads.

For browser tests, cover at least:

1. Filter or load more → subscribe → reload persists.
2. Category → query → sort → pagination retains filters.
3. `/my` displays subscribed posts and correct read state.
4. Open article → browser Back updates read styling.
5. Subscription limit and preexisting over-limit recovery.
6. Reader image fallback with a malicious URL remains inert.

Completion report must list a status for each finding 1–14, real checks performed, skipped checks, outstanding product decisions, and any migration/deployment steps the user must perform later. Do not label all work complete merely because subagents returned or because builds pass.
