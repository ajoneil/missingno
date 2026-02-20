# Wrap-up

End a work session cleanly. Verify the working directory is in a good state, run tests, commit if appropriate, and summarize what was accomplished.

## Scope discipline

**You are a session closer, not an investigator or implementer.** Your job is to:

1. Ensure the working directory is in a known-good state.
2. Run the test suite and report results.
3. Commit changes if tests pass and there are uncommitted changes.
4. Summarize what was accomplished in the session.

You do NOT investigate new issues, start fixing test failures, refactor code, or analyze results. If tests fail, report what failed — do not attempt to fix anything.

## Process

1. **Check git status.** Run `git status` and `git diff --stat` to understand what's changed.
2. **Run the test suite.** Run `./scripts/test-report.sh --diff` to get a structured report with baseline comparison. If no baseline exists yet, run `./scripts/test-report.sh` for a plain report.
3. **Assess results.**
   - If no regressions (newly failing tests that previously passed): proceed to commit.
   - If there are regressions: **stop**. Report the regressions and do NOT commit. The session needs intervention before wrapping up.
4. **Commit.** If there are uncommitted changes and no regressions:
   - Stage the relevant files (not `git add -A` — be specific).
   - Write a clear commit message summarizing the changes.
5. **Update summary.md** (if an active investigation exists). Append a brief note of what was accomplished and the current test state. If there is no active investigation, skip this step.
6. **Report.** Output a short summary:
   - What was committed (or why nothing was committed).
   - Test results (pass/fail counts).
   - Any open issues or regressions noticed.

## Rules

- Do NOT start investigating new issues discovered during wrap-up.
- Do NOT attempt to fix failing tests — just report them.
- Do NOT push to remote unless explicitly asked.
- Do NOT modify code during wrap-up — this is a read-only + commit workflow.
- If the user asked to wrap up mid-investigation, preserve the investigation state as-is. Do not clean up or reorganize investigation files.
