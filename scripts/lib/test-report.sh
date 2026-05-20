#!/usr/bin/env bash
# Shared core for test-report-{gb,gbc}.sh.
#
# Callers (the wrapper scripts) set the following before sourcing:
#   CRATE          — cargo package name (e.g. missingno-gb, missingno-gbc)
#   CRATE_LABEL    — short label used in report titles (e.g. gb, gbc)
#   BASELINE_FILE  — absolute path to this variant's baseline file
#   REPORT_DIR     — absolute path to this variant's report output directory
#   MODE           — "" | --save-baseline | --diff (forwarded from caller's $1)

set -euo pipefail

: "${CRATE:?CRATE must be set}"
: "${CRATE_LABEL:?CRATE_LABEL must be set}"
: "${BASELINE_FILE:?BASELINE_FILE must be set}"
: "${REPORT_DIR:?REPORT_DIR must be set}"
MODE="${MODE:-}"

mkdir -p "$REPORT_DIR"

# Run tests and capture output
RAW_OUTPUT=$(cargo test -p "$CRATE" 2>&1 || true)

# Extract test lines (ok/FAILED/ignored) and sort for stable diffing.
# Uses sed -n /p (always exits 0) so an empty set doesn't trip pipefail.
PASSED=$(echo "$RAW_OUTPUT" | sed -n 's/^[[:space:]]*test \(.*\) \.\.\. ok$/\1/p' | sort)
FAILED=$(echo "$RAW_OUTPUT" | sed -n 's/^[[:space:]]*test \(.*\) \.\.\. FAILED$/\1/p' | sort)
IGNORED=$(echo "$RAW_OUTPUT" | sed -n 's/^[[:space:]]*test \(.*\) \.\.\. ignored$/\1/p' | sort)

PASS_COUNT=$(echo "$PASSED" | grep -c . || true)
FAIL_COUNT=$(echo "$FAILED" | grep -c . || true)
IGNORE_COUNT=$(echo "$IGNORED" | grep -c . || true)

# Build current state file (sorted list of test=status).
# `|| true` absorbs the non-zero exit when a category is empty — the
# `[ -n "$t" ] && echo …` short-circuit returns 1 for the trailing
# empty line, which pipefail + set -e would otherwise treat as fatal.
CURRENT=$(
    {
        echo "$PASSED" | while read -r t; do [ -n "$t" ] && echo "$t=ok"; done
        echo "$FAILED" | while read -r t; do [ -n "$t" ] && echo "$t=FAILED"; done
        echo "$IGNORED" | while read -r t; do [ -n "$t" ] && echo "$t=ignored"; done
    } | sort || true
)

if [ "$MODE" = "--save-baseline" ]; then
    echo "$CURRENT" > "$BASELINE_FILE"
    echo "Baseline ($CRATE_LABEL) saved ($PASS_COUNT passed, $FAIL_COUNT failed, $IGNORE_COUNT ignored)"
    exit 0
fi

# Print report
TIMESTAMP=$(date -Iseconds)
REPORT="# Test Report ($CRATE_LABEL) — $TIMESTAMP

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

    # Extract sorted test-name sets by status. `sed -n 's/=X$//p'`
    # strips the suffix and prints lines that had it.
    baseline_failed=$(echo "$BASELINE" | sed -n 's/=FAILED$//p' | sort)
    baseline_ok=$(echo "$BASELINE" | sed -n 's/=ok$//p' | sort)
    current_failed=$(echo "$CURRENT" | sed -n 's/=FAILED$//p' | sort)
    current_ok=$(echo "$CURRENT" | sed -n 's/=ok$//p' | sort)

    # Newly passing = was FAILED in baseline AND is ok in current.
    NEWLY_PASSING=$(comm -12 <(echo "$baseline_failed") <(echo "$current_ok"))

    # Regressions = was ok in baseline AND is FAILED in current.
    REGRESSIONS=$(comm -12 <(echo "$baseline_ok") <(echo "$current_failed"))

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
