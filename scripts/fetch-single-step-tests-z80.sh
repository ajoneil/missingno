#!/usr/bin/env bash
# Fetch the SingleStepTests Z80 JSON oracle into receipts/resources/.
# Sparse checkout of the v1 set only. Records the fetched commit for provenance.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/receipts/resources/single-step-tests-z80"

if [ -d "$dest/v1" ]; then
    echo "already present: $dest/v1 ($(git -C "$dest" rev-parse --short HEAD))"
    exit 0
fi

git clone --depth 1 --filter=blob:none --sparse \
    https://github.com/SingleStepTests/z80 "$dest"
git -C "$dest" sparse-checkout set v1
git -C "$dest" rev-parse HEAD > "$dest/FETCHED_COMMIT"
echo "fetched $(cat "$dest/FETCHED_COMMIT")"
