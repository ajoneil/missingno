#!/usr/bin/env bash
# Generate a structured test report for missingno-core.
# Usage:
#   ./scripts/test-report.sh                  # Run tests, print report
#   ./scripts/test-report.sh --save-baseline  # Run tests and save as baseline
#   ./scripts/test-report.sh --diff           # Run tests and diff against saved baseline

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BASELINE_FILE="$PROJECT_DIR/scripts/.test-baseline"
REPORT_DIR="$PROJECT_DIR/receipts/test-reports"

mkdir -p "$REPORT_DIR"

# Run tests and capture output
RAW_OUTPUT=$(cargo test -p missingno-core 2>&1 || true)

# Extract test lines (ok/FAILED) and sort for stable diffing
PASSED=$(echo "$RAW_OUTPUT" | grep '^\s*test .* ok$' | sed 's/^\s*test //' | sed 's/ \.\.\. ok$//' | sort)
FAILED=$(echo "$RAW_OUTPUT" | grep '^\s*test .* FAILED$' | sed 's/^\s*test //' | sed 's/ \.\.\. FAILED$//' | sort)
IGNORED=$(echo "$RAW_OUTPUT" | grep '^\s*test .* ignored$' | sed 's/^\s*test //' | sed 's/ \.\.\. ignored$//' | sort)

PASS_COUNT=$(echo "$PASSED" | grep -c . || true)
FAIL_COUNT=$(echo "$FAILED" | grep -c . || true)
IGNORE_COUNT=$(echo "$IGNORED" | grep -c . || true)

# Build current state file (sorted list of test=status)
CURRENT=$(
    (echo "$PASSED" | while read -r t; do [ -n "$t" ] && echo "$t=ok"; done
     echo "$FAILED" | while read -r t; do [ -n "$t" ] && echo "$t=FAILED"; done
     echo "$IGNORED" | while read -r t; do [ -n "$t" ] && echo "$t=ignored"; done) | sort
)

MODE="${1:-}"

if [ "$MODE" = "--save-baseline" ]; then
    echo "$CURRENT" > "$BASELINE_FILE"
    echo "Baseline saved ($PASS_COUNT passed, $FAIL_COUNT failed, $IGNORE_COUNT ignored)"
    exit 0
fi

# Print report
TIMESTAMP=$(date -Iseconds)
REPORT="# Test Report — $TIMESTAMP

## Summary
- **Passed**: $PASS_COUNT
- **Failed**: $FAIL_COUNT
- **Ignored**: $IGNORE_COUNT
"

if [ "$FAIL_COUNT" -gt 0 ]; then
    REPORT+="
## Failing Tests
$(echo "$FAILED" | while read -r t; do [ -n "$t" ] && echo "- $t"; done)
"
fi

# Diff against baseline if requested and baseline exists
if [ "$MODE" = "--diff" ] && [ -f "$BASELINE_FILE" ]; then
    BASELINE=$(cat "$BASELINE_FILE")

    # Find newly passing (was FAILED in baseline, now ok)
    NEWLY_PASSING=$(comm -13 <(echo "$BASELINE" | grep '=ok$' | sort) <(echo "$CURRENT" | grep '=ok$' | sort) | \
        while read -r line; do
            name="${line%=ok}"
            if echo "$BASELINE" | grep -q "^${name}=FAILED$"; then
                echo "$name"
            fi
        done)

    # Find regressions (was ok in baseline, now FAILED)
    REGRESSIONS=$(comm -13 <(echo "$CURRENT" | grep '=ok$' | sort) <(echo "$BASELINE" | grep '=ok$' | sort) | \
        while read -r line; do
            name="${line%=ok}"
            if echo "$CURRENT" | grep -q "^${name}=FAILED$"; then
                echo "$name"
            fi
        done)

    BASELINE_PASS=$(echo "$BASELINE" | grep -c '=ok$' || true)
    BASELINE_FAIL=$(echo "$BASELINE" | grep -c '=FAILED$' || true)
    DELTA=$((PASS_COUNT - BASELINE_PASS))

    REPORT+="
## Baseline Comparison
- **Baseline**: $BASELINE_PASS passed, $BASELINE_FAIL failed
- **Current**:  $PASS_COUNT passed, $FAIL_COUNT failed
- **Delta**: ${DELTA:+$( [ "$DELTA" -ge 0 ] && echo "+$DELTA" || echo "$DELTA" )} tests
"

    if [ -n "$NEWLY_PASSING" ]; then
        REPORT+="
### Newly Passing
$(echo "$NEWLY_PASSING" | while read -r t; do [ -n "$t" ] && echo "- $t"; done)
"
    fi

    if [ -n "$REGRESSIONS" ]; then
        REPORT+="
### Regressions
$(echo "$REGRESSIONS" | while read -r t; do [ -n "$t" ] && echo "- **$t**"; done)
"
    fi

    if [ -z "$NEWLY_PASSING" ] && [ -z "$REGRESSIONS" ]; then
        REPORT+="
No changes from baseline.
"
    fi
fi

echo "$REPORT"

# Save to file
REPORT_FILE="$REPORT_DIR/$(date +%Y-%m-%d-%H%M%S).md"
echo "$REPORT" > "$REPORT_FILE"
echo "---"
echo "Report saved to: $REPORT_FILE"
