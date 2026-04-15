#!/bin/bash
# dmg-sim-observe.sh — Run a ROM through the gate-level DMG simulation
# and dump a waveform (FST) for signal observation.
#
# Usage:
#   ./scripts/dmg-sim-observe.sh <rom_path> [seconds] [output_dir]
#
# Examples:
#   ./scripts/dmg-sim-observe.sh path/to/test.gb 0.5
#   ./scripts/dmg-sim-observe.sh path/to/test.gb 0.3 receipts/investigations/foo/logs
#
# The simulation runs with realistic propagation delays (TIMING=default).
# Output: <output_dir>/<rom_name>.fst (open with GTKWave)
#
# Prerequisites:
#   - iverilog (Icarus Verilog)
#   - receipts/resources/dmg-sim/ cloned

set -euo pipefail

DMG_SIM_DIR="$(cd "$(dirname "$0")/../receipts/resources/dmg-sim" && pwd)"
ROM_PATH="${1:?Usage: $0 <rom_path> [seconds] [output_dir]}"
SECS="${2:-0.5}"
OUTPUT_DIR="${3:-receipts/traces/dmg-sim}"

ROM_NAME="$(basename "$ROM_PATH" .gb)"

# Resolve ROM to absolute path
ROM_PATH="$(cd "$(dirname "$ROM_PATH")" && pwd)/$(basename "$ROM_PATH")"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"

# Compile if needed
if [ ! -f "$DMG_SIM_DIR/dmg_cpu_b_gameboy.vvp" ]; then
    echo "Compiling dmg-sim (first run only)..."
    make -C "$DMG_SIM_DIR" dmg_cpu_b_gameboy.vvp TIMING=default 2>&1 | tail -3
fi

echo "Running gate-level simulation: $ROM_NAME for ${SECS}s..."
echo "  ROM: $ROM_PATH"
echo "  Output: $OUTPUT_DIR/${ROM_NAME}.fst"

cd "$DMG_SIM_DIR"

# Run simulation with FST dump
vvp -N dmg_cpu_b_gameboy.vvp \
    -fst-speed "+DUMPFILE=$OUTPUT_DIR/${ROM_NAME}.fst" \
    +VID_FILE="$OUTPUT_DIR/${ROM_NAME}.vid" \
    +SND_FILE=/dev/null \
    +SAV_FILE=/dev/null \
    +BOOTROM="$DMG_SIM_DIR/boot/quickboot.bin" \
    +ROM="$ROM_PATH" \
    +SECS="$SECS"

FST_SIZE=$(du -sh "$OUTPUT_DIR/${ROM_NAME}.fst" | cut -f1)
echo "Done. FST: $OUTPUT_DIR/${ROM_NAME}.fst ($FST_SIZE)"
echo "Open with: gtkwave $OUTPUT_DIR/${ROM_NAME}.fst"
