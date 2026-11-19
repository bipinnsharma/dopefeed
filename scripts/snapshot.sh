#!/bin/sh
# Publish a fresh snapshot: refresh content, re-encode new images, then push
# the DB and assets. Vercel builds each push from GitHub, so nothing goes
# through the deploy file-upload API. Run weekly or on demand.
set -eu
cd "$(dirname "$0")/.."

cargo run --release -- refresh
python3 scripts/topfeed_sync.py --apply
cargo run --release -- optimize-images
cargo run --release -- verify-assets

# .backup reads through the WAL, so a running dev server cannot leave posts out
# of the snapshot the way a busy wal_checkpoint does.
sqlite3 data/dopefeed.db ".backup data/dopefeed.snapshot.db"
xz -9 -f data/dopefeed.snapshot.db
mv -f data/dopefeed.snapshot.db.xz data/dopefeed.db.xz

git add -A
if git diff --cached --quiet; then
    echo "nothing changed; no deploy"
    exit 0
fi
git commit -m "Snapshot $(date +%Y-%m-%d)"
git push origin "$(git branch --show-current)"
