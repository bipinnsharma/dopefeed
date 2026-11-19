#!/bin/sh
# Local only: the DB snapshot and static/assets are gitignored, so Vercel must
# be deployed from this working copy. Run weekly or on demand.
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

# A stale CLI token is the usual cause of a deploy dying at 0.0B.
vercel whoami
vercel deploy --prod --yes
