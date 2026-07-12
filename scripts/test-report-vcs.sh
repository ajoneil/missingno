#!/usr/bin/env bash
# Generate a structured test report for missingno-vcs (Atari VCS core).
# Usage:
#   ./scripts/test-report-vcs.sh                  # Run tests, print report
#   ./scripts/test-report-vcs.sh --save-baseline  # Run tests and save as baseline
#   ./scripts/test-report-vcs.sh --diff           # Run tests and diff against saved baseline

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

export CRATE="missingno-vcs"
export CRATE_LABEL="vcs"
export BASELINE_FILE="$PROJECT_DIR/scripts/.test-baseline-vcs"
export REPORT_DIR="$PROJECT_DIR/receipts/test-reports/vcs"
export MODE="${1:-}"

# shellcheck source=lib/test-report.sh
source "$SCRIPT_DIR/lib/test-report.sh"
