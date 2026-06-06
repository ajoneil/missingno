#!/usr/bin/env bash
# Profile-guided-optimization build: instrument the bench examples, train on
# the in-repo (redistributable) test ROMs across both cores, then rebuild the
# workspace with the merged profile.
#
#   ./scripts/pgo-build.sh                 # PGO build of the whole workspace (release)
#   PROFILE=profiling ./scripts/pgo-build.sh
#   TRAIN_FRAMES=300 ./scripts/pgo-build.sh
#   PGO=0 ./scripts/pgo-build.sh           # escape hatch: plain build, no PGO
#
# Requires llvm-profdata from the toolchain's sysroot (rustup: the llvm-tools
# component; the Flatpak rust-stable SDK extension ships it already).
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${PROFILE:-release}"
TRAIN_FRAMES="${TRAIN_FRAMES:-600}"
PGO_DIR="$(pwd)/target/pgo-profiles"

if [ "${PGO:-1}" = "0" ]; then
    echo "== PGO disabled: plain build =="
    cargo build --profile "$PROFILE"
    exit 0
fi

GB_ROMS=crates/missingno-gb/tests/accuracy/roms
GBC_ROMS=crates/missingno-gbc/tests/accuracy/roms

# Training set: spread across both cores — PPU-heavy (acid), CPU-heavy
# (cpu_instrs), APU-heavy (dmg_sound), and KEY1 double-speed (-ds) workloads.
TRAIN_DMG=(
    "$GB_ROMS/dmg-acid2/dmg-acid2.gb"
    "$GB_ROMS/blargg/cpu_instrs/cpu_instrs.gb"
    "$GB_ROMS/blargg/dmg_sound/dmg_sound.gb"
)
TRAIN_CGB=(
    "$GBC_ROMS/cgb-acid2/cgb-acid2.gbc"
    "$GBC_ROMS/cgb-acid-hell/cgb-acid-hell.gbc"
    "$GBC_ROMS/age-test-roms/m3-bg-lcdc-ds.gb"
    "$GBC_ROMS/age-test-roms/stat-mode-sprites-ds-cgbBCE.gb"
)

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LLVM_PROFDATA="$(rustc --print sysroot)/lib/rustlib/$HOST_TRIPLE/bin/llvm-profdata"
[ -x "$LLVM_PROFDATA" ] || LLVM_PROFDATA=llvm-profdata

echo "== Stage 1/3: instrumented build =="
rm -rf "$PGO_DIR"
mkdir -p "$PGO_DIR"
RUSTFLAGS="-Cprofile-generate=$PGO_DIR" \
    cargo build --profile "$PROFILE" -p missingno-gb -p missingno-gbc --examples

echo "== Stage 2/3: training ($TRAIN_FRAMES frames per ROM) =="
for rom in "${TRAIN_DMG[@]}"; do
    echo "  train(dmg): $rom"
    "target/$PROFILE/examples/bench-dmg" "$rom" "$TRAIN_FRAMES"
done
for rom in "${TRAIN_CGB[@]}"; do
    echo "  train(cgb): $rom"
    "target/$PROFILE/examples/bench-gbc" "$rom" "$TRAIN_FRAMES"
done
"$LLVM_PROFDATA" merge -o "$PGO_DIR/merged.profdata" "$PGO_DIR"/*.profraw

echo "== Stage 3/3: optimized build =="
RUSTFLAGS="-Cprofile-use=$PGO_DIR/merged.profdata" \
    cargo build --profile "$PROFILE"

echo "PGO build complete: target/$PROFILE/missingno"
echo "To build other targets against this profile:"
echo "  RUSTFLAGS=\"-Cprofile-use=$PGO_DIR/merged.profdata\" cargo build --profile $PROFILE <args>"
