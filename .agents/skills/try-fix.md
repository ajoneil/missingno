# Try-Fix

Run a bounded fix-validate-revert loop. Apply a design, measure the result against a baseline, and automatically revert on regression. Return a structured report of what was tried and what happened so the caller can decide the next step.

## Scope discipline

**You are an executor with a revert trigger, not a designer or investigator.** You receive a design and a baseline. You apply the design, run the test suite, compare against the baseline, and either keep or revert. If the fix doesn't work within the attempt limit, you stop and report — you do not redesign, hypothesize, or analyze.

You do NOT:
- Redesign the solution mid-loop
- Hypothesize about why a fix failed
- Analyze test output beyond pass/fail/regression counting
- Tweak the design between attempts (each attempt applies the same design — if it regresses, it's reverted and the loop ends)
- Expand scope to fix adjacent issues discovered during verification

**One design, one loop.** If the caller wants to try a different design, that's a new invocation.

## Inputs

The caller provides:

- **Design**: Path to the design receipt file, OR a brief inline description for a minimal targeted change.
- **Target test**: The specific test(s) to improve (e.g., `mooneye::ppu_intr_2_mode0_timing_sprites`).
- **Baseline**: Current test counts (passed/failed) or path to a baseline report. If not provided, establish one in step 1.
- **Attempt limit**: Maximum number of attempts before returning. Default: 3.
- **Context**: Path to the investigation's `summary.md` (if called from an investigation).

## Process

### 1. Establish baseline

If the caller didn't provide baseline counts:

```bash
cargo test -p missingno-core 2>&1 | tee <log_path>
```

Record:
- Total passed / failed / ignored
- Target test result (pass/fail, mismatch count if applicable)
- List of currently-passing tests (the regression reference set)

Write the baseline to the top of the report file immediately.

### 2. Create a working branch

```bash
git stash --include-untracked  # if working dir is dirty
git checkout -b try-fix/<short-name>
```

This branch is disposable. All work happens here.

### 3. Apply the design

Invoke `/implement` with the design, or make the changes directly if the design is a small inline description. The changes must compile (`cargo check`).

If the changes don't compile, record the error, revert, and count it as a failed attempt.

### 4. Measure

Run the full core test suite and capture output:

```bash
cargo test -p missingno-core 2>&1 | tee <log_path>
```

Extract:
- Target test result (pass/fail, mismatch count)
- Total passed / failed
- Any previously-passing tests that now fail (regressions)

### 5. Decide: keep or revert

**Keep** if ALL of these are true:
- Target test improved (mismatch count decreased, or changed from fail to pass)
- No regressions (no previously-passing test now fails)

**Revert** if ANY of these are true:
- Target test got worse or stayed the same
- Any regression appeared (even if target test improved)

On revert:
```bash
git checkout -- .
git clean -fd
```

Record the attempt result before reverting.

### 6. Loop or stop

- If kept: commit the changes, record success, stop.
- If reverted and attempts remain: the loop ends. **Do not retry the same design.** A design that regresses once will regress again. The remaining attempts exist for the caller to invoke try-fix again with a *different* design. Record the failure and return to the caller.
- If reverted and no attempts remain: record the failure and return to the caller.

**Important:** This means each try-fix invocation makes exactly one attempt with the given design. The attempt limit is tracked across invocations within the same investigation, not within a single invocation. The caller is responsible for invoking try-fix with different designs up to the limit.

### 7. Clean up

After the loop ends (success or exhaustion):

- If successful: merge the working branch back to the base branch, delete the working branch.
- If failed: return to the base branch, leave the working branch for reference.
- Remove any diagnostic logging added during the process.
- Run `cargo fmt`.

## Output

Write a report to the investigation's folder (or receipts root if standalone):

```
receipts/investigations/<session>/try-fix/<NNN>-<short-name>.md
```

### Report format

```markdown
# Try-Fix: <short title>

## Baseline
- Passed: <N> | Failed: <N>
- Target test: <name> — <result> (mismatch count: <N> if applicable)

## Attempt <N>

### Design applied
<path to design receipt or inline description>

### Changes made
<brief summary of what was changed>

### Result
- Target test: <improved / same / worse> (before: <X>, after: <Y>)
- Regressions: <none / list of regressed tests>
- Verdict: **kept** / **reverted**

### Log
<path to test output log>

## Summary
- Attempts used: <N> of <limit>
- Outcome: **success** / **reverted — no improvement** / **reverted — regression**
- Branch: <branch name> (merged / preserved / deleted)

## Observations
<Factual observations only — no interpretation, no recommendations.
What changed in the test output. Which tests were affected.
The caller decides what this means.>
```

## After try-fix is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the try-fix report to the receipt file.
2. **Do not update `summary.md`.** The caller (investigate) owns summary.md.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller.

**The turn does not end here.** Do NOT stop after writing the report. The caller must act on the result in the same turn.
