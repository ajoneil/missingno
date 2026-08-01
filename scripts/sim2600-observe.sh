#!/usr/bin/env bash
# sim2600-observe.sh — Run an Atari 2600 ROM through the gate-level Sim2600
# transistor simulation (6507 + TIA, from the visual6502 decapped netlists)
# and dump a VCD waveform of named wires per half-clock for signal observation.
#
# The VCS analog of dmg-sim-observe.sh. Use it to answer *why* the CPU or the
# beam does what it does at a specific wire and half-clock.
#
# Usage:
#   ./scripts/sim2600-observe.sh <rom> [half_clocks] [output_dir] [extra_wires]
#
# Examples:
#   ./scripts/sim2600-observe.sh crates/systems/vcs/tests/accuracy/roms/harness/sanity_ntsc.a26 3000
#   ./scripts/sim2600-observe.sh path/to/game.a26 45000 receipts/traces/sim2600 cpu:SYNC,tia:BL_lowCtrl
#
# Output: <output_dir>/<rom_name>.vcd  (open with GTKWave)
#
# IMPORTANT: Sim2600's 6532/RIOT is a behavioural emulation (emuPIA), NOT a
# netlist — its state is not gate-level ground truth. Observe CPU and TIA wires
# here; ground RIOT timing on the datasheet/schematics (crates/systems/vcs/AGENTS.md).
#
# Sim2600 is transistor-level and slow: ~40000 half-clocks before the first
# visible pixel (minutes). Default is 2000 half-clocks (reset + early execution).
#
# Prerequisites: python3, git.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SIM_DIR="$PROJECT_DIR/receipts/resources/Sim2600"

ROM_PATH="${1:?Usage: $0 <rom> [half_clocks] [output_dir] [extra_wires]}"
HALF_CLOCKS="${2:-2000}"
OUTPUT_DIR="${3:-$PROJECT_DIR/receipts/traces/sim2600}"
EXTRA_WIRES="${4:-}"

ROM_NAME="$(basename "$ROM_PATH")"
ROM_NAME="${ROM_NAME%.*}"
ROM_PATH="$(cd "$(dirname "$ROM_PATH")" && pwd)/$(basename "$ROM_PATH")"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"

# Clone Sim2600 on first use.
if [ ! -d "$SIM_DIR" ]; then
    echo "Cloning Sim2600 into receipts/resources/ (first run only)..."
    git clone --depth 1 https://github.com/gregjames/Sim2600 "$SIM_DIR"
fi

# Port the Py2-era package to Python 3, once (2to3 is gone in 3.13+, so apply
# the enumerated edits directly). Guarded by a marker so re-runs are no-ops.
if [ ! -f "$SIM_DIR/.py3-ported" ]; then
    echo "Applying Python 3 port to Sim2600 (first run only)..."
    python3 - "$SIM_DIR" <<'PORT'
import sys, os
d = sys.argv[1]
def patch(fname, repls):
    p = os.path.join(d, fname)
    s = open(p, encoding="latin1").read()
    for old, new in repls:
        if old in s:
            s = s.replace(old, new)
        elif new not in s:  # neither pre- nor post-state present => typo
            raise SystemExit("port: %r not found in %s" % (old, fname))
    open(p, "w", encoding="latin1").write(s)

patch("circuitSimulatorBase.py", [
    ("print 'ERROR - trying to set wire None high'",
     "print('ERROR - trying to set wire None high')"),
    ("print 'ERROR - trying to set wire None low'",
     "print('ERROR - trying to set wire None low')"),
    ("print 'Loading %s' % filePath", "print('Loading %s' % filePath)"),
    ("in xrange(", "in range("),
    ("d = d / 2", "d = d // 2"),
    ("rootObj = pickle.load (of)", "rootObj = pickle.load (of, encoding='latin1')"),
])
patch("sim6502.py", [("in xrange(", "in range(")])
patch("simTIA.py", [("in xrange(", "in range(")])
# Py3: iterating bytes yields ints, so struct.unpack on a single byte breaks.
patch("sim2600Console.py", [("intVal = struct.unpack ('1B', byte)[0]", "intVal = byte")])
print("  ported circuitSimulatorBase.py, sim6502.py, simTIA.py, sim2600Console.py")
PORT
    touch "$SIM_DIR/.py3-ported"
fi

echo "Running Sim2600: $ROM_NAME for $HALF_CLOCKS half-clocks..."
echo "  ROM:    $ROM_PATH"
echo "  Output: $OUTPUT_DIR/${ROM_NAME}.vcd"

# The driver imports the Sim2600 package (PYTHONPATH) and reads chips/*.pkl by
# relative path (cwd), so give it both.
( cd "$SIM_DIR" && PYTHONPATH="$SIM_DIR" python3 "$SCRIPT_DIR/sim2600_observe.py" \
    --rom "$ROM_PATH" \
    --half-clocks "$HALF_CLOCKS" \
    --out "$OUTPUT_DIR/${ROM_NAME}.vcd" \
    --extra-wires "$EXTRA_WIRES" )

echo "Done. Open with: gtkwave $OUTPUT_DIR/${ROM_NAME}.vcd"
