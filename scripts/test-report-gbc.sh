#!/usr/bin/env bash
# Generate a structured test report for missingno-gbc (CGB core).
# Usage:
#   ./scripts/test-report-gbc.sh                  # Run tests, print report
#   ./scripts/test-report-gbc.sh --save-baseline  # Run tests and save as baseline
#   ./scripts/test-report-gbc.sh --diff           # Run tests and diff against saved baseline

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

export CRATE="missingno-gbc"
export CRATE_LABEL="gbc"
export BASELINE_FILE="$PROJECT_DIR/scripts/.test-baseline-gbc"
export REPORT_DIR="$PROJECT_DIR/receipts/test-reports/gbc"
export MODE="${1:-}"

# shellcheck source=lib/test-report.sh
source "$SCRIPT_DIR/lib/test-report.sh"
