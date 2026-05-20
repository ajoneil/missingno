#!/usr/bin/env bash
# Generate a structured test report for missingno-gb (DMG core).
# Usage:
#   ./scripts/test-report-gb.sh                  # Run tests, print report
#   ./scripts/test-report-gb.sh --save-baseline  # Run tests and save as baseline
#   ./scripts/test-report-gb.sh --diff           # Run tests and diff against saved baseline

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

export CRATE="missingno-gb"
export CRATE_LABEL="gb"
export BASELINE_FILE="$PROJECT_DIR/scripts/.test-baseline-gb"
export REPORT_DIR="$PROJECT_DIR/receipts/test-reports/gb"
export MODE="${1:-}"

# shellcheck source=lib/test-report.sh
source "$SCRIPT_DIR/lib/test-report.sh"
