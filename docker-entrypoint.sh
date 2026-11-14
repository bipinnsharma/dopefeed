#!/bin/sh
set -eu
# Vercel containers can only write under /tmp, so seed that copy from the
# bundled snapshot. Writes are lost when the instance scales to zero.
if [ ! -f /tmp/dopefeed.db ]; then
  xz -dc /app/seed/dopefeed.db.xz > /tmp/dopefeed.db
fi
exec dopefeed
