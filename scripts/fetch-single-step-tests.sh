#!/usr/bin/env bash
# Fetch the SingleStepTests 65x02 JSON oracle into receipts/resources/.
# Sparse checkout of the 6502 set only (the full repo carries every 65x02
# variant). Records the fetched commit for provenance.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/receipts/resources/single-step-tests"

if [ ! -d "$dest" ]; then
    git clone --depth 1 --filter=blob:none --sparse \
        https://github.com/SingleStepTests/65x02 "$dest"
fi
git -C "$dest" sparse-checkout set 6502/v1 nes6502/v1
git -C "$dest" rev-parse HEAD > "$dest/FETCHED_COMMIT"
echo "fetched $(cat "$dest/FETCHED_COMMIT")"
