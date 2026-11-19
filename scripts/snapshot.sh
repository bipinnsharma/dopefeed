#!/bin/sh
# Local only: the DB snapshot and static/assets are gitignored, so Vercel must
# be deployed from this working copy. Run weekly or on demand.
set -eu
cd "$(dirname "$0")/.."

cargo run --release -- refresh
python3 scripts/topfeed_sync.py --apply
cargo run --release -- optimize-images
cargo run --release -- verify-assets
# Upload any new or re-encoded image. Images are served from Blob, not the
# deployment, which keeps the deploy under Vercel's file-upload quota.
cargo run --release -- sync-assets

# Hardcoded default DATABASE_URL; change if you point the app elsewhere.
sqlite3 data/dopefeed.db "PRAGMA wal_checkpoint(TRUNCATE);"
xz -9 -f -k data/dopefeed.db
vercel deploy --prod
