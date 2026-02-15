# Implement

Apply a design to the codebase — make the code changes, verify them, and report the result.

## Scope discipline

**You are an implementer, not a designer or investigator.** You receive a design receipt that specifies what to change and where. Your job is to:

1. Read and understand the design.
2. Read the code that needs to change.
3. Make the changes described in the design.
4. Verify the changes with the test suite.
5. Report whether the implementation succeeded or failed.

You do NOT redesign the solution, investigate root causes, hypothesize about problems, or decide what to do next. If the design is ambiguous or insufficient, report that — do not fill in the gaps yourself.

**Implement exactly what the design says.** If the design says "add a constant `FOO = 4`", add that constant. Do not second-guess the value, rename it, or add additional constants "for completeness". If the design says to change a formula, change exactly that formula — do not refactor surrounding code, add comments to unchanged lines, or "improve" adjacent logic.

**Do not design while implementing.** If you catch yourself writing sentences like "actually, it would be better to..." or "we also need to handle..." or "while we're here, let's also..." — stop. You are designing, not implementing. Make the changes the design specifies, verify them, and report. If the design is incomplete, that's a finding to report, not a gap to fill.

## Inputs

The caller provides:

- **Design**: Path to the design receipt file. Read it — this is your specification.
- **Context**: Path to the investigation's `summary.md` (if called from an investigation). Read it for background, but implement only what the design says.

## Process

### 1. Read the design

Read the design receipt file completely. Identify:
- Which files need to change
- What the changes are (new code, modified code, removed code)
- What the state model is (enums, structs, transitions)
- What edge cases the design identifies
- What risks the design flags

### 2. Read the code

Read each file that the design says to modify. Understand the existing code structure so you can make changes that fit naturally. Read only what you need — if the design says to change one function, read that function and its immediate context, not the entire file.

### 3. Make the changes

Apply the design's changes to the code. Follow these rules:

- **One logical change at a time.** If the design specifies changes to three files, make them in a coherent order — usually the order the design presents them.
- **Match existing style.** Use the same indentation, naming conventions, comment style, and patterns as the surrounding code. Do not introduce new conventions.
- **Preserve correctness.** Every intermediate state should compile. If changes span multiple files, make them in an order that keeps the build passing (or at least compiling) at each step.
- **No extras.** Do not add docstrings, comments, type annotations, error handling, or refactoring beyond what the design specifies. Do not "clean up" code near the changes. The diff should contain exactly the design's changes and nothing else.
- **Named constants over magic numbers.** If the design specifies a named constant, use it. If the design uses a magic number, use the magic number — but flag it in your report if it violates the project's architectural principles (the caller can decide whether to invoke `/design` again).

### 4. Verify

After all changes are made:

- **Build check**: Run `cargo check` to verify compilation.
- **Targeted test**: If the design or caller specifies a particular test, run it.
- **Full suite**: Run `cargo test -p missingno-core` to check for regressions.

Capture all test output to a log file:
```
receipts/investigations/<session>/logs/<NNN>-implementation-verify.log
```

Use the next sequential log number in the investigation's logs folder.

**Always tee to a log file.** Never run `cargo test` without capturing output. Use:
```bash
cargo test -p missingno-core 2>&1 | tee <log_path>
```

### 5. Assess the result

Compare the test results against the baseline (from `summary.md`):

- **Success**: The targeted test(s) pass AND no new regressions (failure count does not increase). Report the before/after counts.
- **Partial success**: Some targeted tests pass but not all, or some regressions appeared. Report exactly which tests changed status.
- **Failure**: Targeted tests still fail, or new regressions appeared. Report the failure count and which tests regressed.

**Do not interpret why a failure occurred.** Report the test results — which tests pass, which fail, how that compares to baseline. The caller will decide whether to invoke `/analyze`, `/hypothesize`, or `/design` based on the results.

### 6. Clean up

- **Remove all diagnostic logging** (`eprintln!`, `dbg!`, diagnostic print statements) that was added during the investigation. The implementation should contain only production code.
- **Run `cargo fmt`** to ensure formatting is consistent.
- **Run `cargo clippy`** to catch any lint issues introduced by the changes.

### 7. Branch and commit

Every implementation gets its own branch. This preserves state — even failed attempts are recoverable from their branch.

**Before making any code changes** (step 3), record the current branch and create a new one:

```bash
# Record the base branch (report this in the receipt)
git branch --show-current  # e.g., "main" or "write-conflict-flush-fix"

# Create an implementation branch from the current state
git checkout -b impl/<short-name>  # e.g., "impl/reduce-pre-write-flush"
```

Use a descriptive kebab-case name matching the implementation receipt name. If the branch already exists (from a prior attempt), append a number: `impl/reduce-pre-write-flush-2`.

**After verification**, commit all changes to the implementation branch regardless of outcome (success, partial success, or failure):

```bash
git add -A && git commit -m "<short summary of what changed>"
```

The commit message should be a concise one-liner describing the code change (not the investigation context). Examples: "Use variant N directly as pre-write flush count", "Add sprite fetch state machine", "Fix STAT interrupt blocking during mode transitions".

**After committing**, switch back to the base branch:

- **On success (no regressions):** merge the implementation branch into the base branch, then continue on the base branch:
  ```bash
  git checkout <base-branch>
  git merge impl/<short-name>
  ```
- **On failure or regression:** return to the base branch without merging. The implementation branch stays around as a record:
  ```bash
  git checkout <base-branch>
  ```

**Report the branch name** in the implementation receipt so the caller can find it later if needed.

**Do not push.** The implement skill works locally. The caller (investigate) decides when to push.

## Output

Write an implementation report. The location depends on context:

**When called from an investigation:** Write to the investigation's folder:
```
receipts/investigations/<session>/implementation/<NNN>-<short-name>.md
```

Create the `implementation/` directory if it doesn't exist.

### Report format

```markdown
# Implementation: <short title>

## Design applied
<path to the design receipt>

## Changes made

### <file path>
<description of what was changed — not the full diff, but enough to understand the modification>

### <file path>
...

## Verification

### Build
<cargo check result — pass/fail>

### Tests
<test results — pass count, fail count, comparison to baseline>
<log file path>

## Branch
<base branch> → <implementation branch>
<merged / not merged>

## Result
<success / partial success / failure — one word, then explanation>

## Issues encountered
<any problems during implementation — ambiguities in the design, unexpected compilation errors, code that didn't match the design's assumptions. "None" if everything went smoothly.>
```

## When the design doesn't match the code

Sometimes the design makes assumptions about the code structure that turn out to be wrong — a function signature differs, a field doesn't exist, an enum variant has different data. When this happens:

1. **Do not improvise.** Do not guess what the design "meant" and implement something different.
2. **Make the minimal adaptation** if the mismatch is trivial (e.g., the design says `field_name` but the code uses `field_name_`). Document the adaptation in "Issues encountered".
3. **Stop and report** if the mismatch is structural (e.g., the design assumes a state machine that doesn't exist, or a method that takes different parameters). The caller needs to re-invoke `/design` with updated information.

## After implementation is complete

This skill is a subroutine — see "Subroutine discipline" in the skill invocation protocol in AGENTS.md.

1. Write the implementation report to the receipt file.
2. **Do not update `summary.md`.** The caller (investigate) owns summary.md and will incorporate the result into the RCA tree.
3. **Resume as the caller.** Read the return context block from summary.md, re-read the caller's skill file, delete the "Active subroutine" section, and immediately continue working as the caller. **Do not decide what to do next** — the caller reads the implementation report and makes that decision.

**The turn does not end here.** Do NOT stop after writing the report. The caller must act on the result in the same turn.
