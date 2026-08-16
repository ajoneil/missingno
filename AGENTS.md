# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Skill System Rules

These are the top-level rules governing how skills interact. They survive context compaction and override any default agent behavior.

1. **Always use skills — never ad-hoc.** When asked to investigate, debug, research, or analyze, invoke the appropriate skill (`/investigate`, `/research`, `/compare-traces`, etc.). Never start ad-hoc analysis, use WebSearch directly, read reference emulator source yourself, or trace behavior in your head. The skill system exists to enforce scope discipline and produce durable receipts. Bypassing it produces unreliable, unreproducible results that don't survive context compaction.

2. **Hardware is the source of truth.** The goal is always to understand what the real hardware does and model that behavior. Prioritize hardware documentation, decaps, test measurements, and direct hardware observations over any emulator implementation. Other emulators (SameBoy, Gambatte, DocBoy, etc.) are reference material — useful for confirming hardware behavior, but never the primary source and never a model to copy. The question is always "what does the hardware do?" not "what does emulator X do?" Each core's ground-truth hierarchy differs and lives beside its crate (see *Per-core methodology*) — read the core's doc before any accuracy work there. The rule holds everywhere, but what operationalises it (gate-level sim, hardware captures, test-ROM values) is per-core, and CGB and the VCS RIOT have no gate-level source at all.

3. **Skills are subroutine calls — never stopping points.** When a subagent skill returns, or an in-context skill exits, the caller MUST immediately read the receipt, update `summary.md`, and continue the investigation in the same turn. Never end your turn after a skill produces its receipt. Skill invocations are function calls, not async tasks you wait on.

4. **summary.md is the single source of truth for investigation state.** Update it before every skill dispatch and after every skill return — no exceptions. If context were compacted right now, `summary.md` alone must tell you exactly where you are and what to do next. summary.md is owned by the `/investigate` dispatcher, not by skills — when an in-context skill exits, you exit its mode first and then update summary.md as the dispatcher.

5. **Use available data before generating new data.** Before instrumenting code or running the debugger, check whether the question can be answered with existing resources. The **ordered reference hierarchy is per-core** — it lives in that core's methodology doc (`crates/systems/<core>/AGENTS.md`; see *Per-core methodology* below) because each core's ground truth differs. Read the target core's hierarchy first, and also check **existing research** (`receipts/research/`) and prior investigations. Generate new diagnostic data only when these existing sources don't answer the question.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored. **Never reference receipt paths in committed code or committed docs** (comments, commit messages, AGENTS.md resource references, etc.) — receipts are ephemeral working documents that do not travel between checkouts. Sources in a methodology doc get a URL or "no public URL located", never a local-copy path.
- **`receipts/resources/`** — External resources: sibling projects, reference emulator source, hardware schematics, etc. Clone or download whatever you need into this directory. It's gitignored, so treat it as a workspace for external material.
- **Long-running efforts** keep a spine at `receipts/<effort>/ROADMAP.md` (decisions, phase status, gates). Before starting work that might belong to an active effort, check for such a spine and read it first.

### Context hygiene

The conversation context is volatile — it will be compacted unpredictably. Treat files on disk as the primary memory; conversation is scratch space.

- **Write early, write often.** After every meaningful step, write it to the appropriate file. Test: if context were compacted right now, would your progress survive?
- **Keep context lean.** After writing state to disk, re-read the file when you need the information — don't carry it in conversation memory.
- **After context compaction**: Re-read the active skill file from `.agents/skills/` and `summary.md` before continuing. The compaction summary won't preserve skill directives.
- **Periodically during long sessions**: Every ~10 tool calls, re-read the active skill file and `summary.md` to catch drift.

### Skill invocation protocol

Skills invoke other skills as subroutines. There are two execution flavors:

**Subagent skills** — `/research`, `/analyze`, `/instrument`, `/inspect`, `/compare-traces`, `/test-report`. These are fact-finding tasks that produce large diagnostic outputs (file reads, source exploration, measurement data, test output). They run as Task subagents (`subagent_type: "general-purpose"`) so that intermediate work stays out of the main context window. Each subagent receives its skill file in the Task prompt, writes a receipt file, and stops. It does NOT inherit the caller's context or hypotheses.

**In-context skills** — `/hypothesize`, `/design`, `/implement`. These are synthesis tasks where conversation continuity (prior reasoning, the user's clarifications, mid-flight course corrections) is load-bearing. They run on the main agent. Before invoking, re-read the skill file from `.agents/skills/<skill>.md` to load its scope discipline, then switch into that mode for the duration. The scope-discipline rules are critical — the main agent must follow them as strictly as a subagent would, since the only thing keeping you honest is the skill file itself.

For both flavors, the caller owns interpretation and decision-making across skill boundaries; the callee owns its scoped task. Only `/investigate` makes decisions about what to do next — all other skills produce a receipt and exit. Both flavors require the same Question/Context brief in summary.md before invocation.

#### Request format (caller → callee)

```
**Question**: <one specific, concrete, testable question — one sentence>
**Context**: <only what the callee needs — file paths, subsystem names, output location>
**Log path**: <where to save command output> (instrument only)
```

Do NOT include: caller's hypotheses, diagnostic output from prior steps, reasoning about what the answer means, or multiple unrelated questions.

For subagent skills the brief goes into the Task prompt. For in-context skills the brief still goes in summary.md (or scratch) — writing it forces the same clarity even though no subagent reads it.

#### Report format (callee → caller)

Reports must contain only facts and measurements — no interpretation, recommendations, or analysis. Research reports use: Findings / Sources / Confidence / See also. Instrument reports use: Test result / Measurements / Raw data / Also observed. If a sentence starts with "This means..." or "The fix should be..." — delete it.

In-context skills produce their own format per the skill file (designs use State model / Changes by file / etc.; hypotheses use the ranked list format; implementations use the Changes / Verification / Result format). The "facts only" discipline still applies — interpretation belongs in summary.md, not in the skill receipt.

## Workflow Discipline

- When asked to update documentation, commit, or do a simple task, do exactly that — don't go on analysis tangents or start investigating new issues.
- Before starting any work, check git status and ensure the working directory is clean. If there are uncommitted changes or stashed work, ask before proceeding.
- If the Read tool returns stale content (especially after git operations), fall back to `cat <file>` via Bash.

## Project Overview

Missingno is a Rust emulator and debugger. Its mature core is the Game Boy family (DMG + Game Boy Color); additional console cores (Atari VCS, Sega SG-1000, and first-pass SMS and NES) share the app, debugger, and skill infrastructure. Each core's accuracy methodology lives beside its crate (see *Per-core methodology*).

## Build and Run Commands

**Do not use `--release` unless explicitly asked.** Release builds are slow to compile. Debug builds are the default for development, testing, and debugging. Standing exception: benchmarking and profiling are release-build work — the bench examples (`cargo run -p missingno-gb --example bench-dmg --release -- <rom> [frames]`, likewise `bench-gbc` in missingno-gbc) and `./scripts/pgo-build.sh` (PGO is the release/shipping configuration; trains on in-repo ROMs via the bench examples; needs the rustup `llvm-tools` component). Never benchmark with other builds/suites/agents running on the box.

```bash
cargo run                                    # Build and run (debug)
cargo run -- path/to/rom.gb                  # Load a ROM
cargo run -- path/to/rom.gb --debugger       # Load with debugger
cargo run -- path/to/rom.gb --boot-rom path/to/dmg_boot.bin  # Run with boot ROM
cargo run -p missingno-debugger -- path/to/rom.gb            # Headless debugger (HTTP API)
cargo check                                  # Type check
cargo test -p missingno-gb                 # Run core tests (fast, no GUI deps)
cargo test                                   # Run default-member tests (shipping crates; NES/SMS need -p)
cargo clippy                                 # Lint
cargo fmt                                    # Format
```

## Testing

- Run tests against the crate you're working on: `cargo test -p missingno-gb` (DMG), `-p missingno-gbc` (CGB), `-p missingno-vcs` (Atari VCS), `-p missingno-sg1000` (SG-1000), and the chip crate behind whatever you touched (`-p missingno-zilog-z80`, `-p missingno-mos-6502`, `-p missingno-ti-vdp`, `-p missingno-ti-psg`). Do not run `cargo test` against the whole workspace unless specifically asked.
- **The gate for any change is a fully-passing suite for every crate it touches; ANY failure is a regression.** This holds for every core and every chip crate, not a privileged subset. The VCS references are hardware-endorsed (see its methodology doc), so a VCS screenshot regression is a divergence from measured silicon.
- For regression checking on the cores that have one, use `./scripts/test-report-gb.sh --diff` instead of raw `cargo test`. It generates structured reports with baseline comparison and saves them to `receipts/test-reports/gb/`. Variants: `./scripts/test-report-gbc.sh` (reports under `receipts/test-reports/gbc/`) and `./scripts/test-report-vcs.sh` (`receipts/test-reports/vcs/`); use the one matching the core you're changing. There is no report script for the SG-1000 or the chip crates — run plain `cargo test -p <crate>` there. The report scripts exist for accuracy investigations and regression triage; for ordinary changes, plain `cargo test` under `timeout` suffices anywhere.
- Wrap suite runs in a timeout after disruptive changes (e.g. `timeout 1500 ./scripts/test-report-gb.sh --diff`; normal full-suite runtime is ~6 min). Interrupt/halt-adjacent changes can hang the emulator — treat a large overrun as a hang: kill it, then bisect with single-test probes (build with `--no-run` first, then `timeout 60-90` on the run so the timeout does not catch the rebuild).
- To save a baseline before experimenting: `./scripts/test-report-gb.sh --save-baseline` (or the `-gbc` variant). Always save a baseline from `main` (or the known-good state) before making changes, so `--diff` has an accurate reference point.
- **The session crate's gate is `cargo test -p missingno-session --all-features`.** Its default features leave `tools` off (so an embedder links no JSON/PNG codec), and that silently skips every tool-surface and attach test — a plain `cargo test -p missingno-session` passes while those regress. Pair it with `cargo test -p missingno-debugger --features mcp` for the transports.
- To run a specific test with the boot ROM: `DMG_BOOT_ROM=<path> cargo test -p missingno-gb <test_name>`. Boot ROMs are proprietary — ask the user for the path, never commit them. Only use on targeted tests; the boot ROM adds significant startup time per test, making full-suite runs impractical.
- After any fix, verify no regressions before committing.

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Three outcomes, no fourth**: A divergence resolves as a mechanistic fix derived from structure, an honest abstraction with its limits stated, or a conscious decline, documented. Fitting a constant until a test passes is none of them.
- **"Approximation artifact" is an inadmissible classification**: A divergence attributed to the model's own resolution is a representational error, not an approximation. Fix it structurally, or own the coarser quantum as a stated abstraction limit — never leave it classified as rounding.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
- **Evidence is external, named, and tiered**: Ground truth is a named external artifact — a netlist, decap, capture, datasheet, or hardware measurement. Each core ranks its own tiers in its methodology doc; a finding carries the tier that backs it. Self-consistency and plausibility are not evidence, however thin a core's tiers run.
- **Hardware over emulator comparisons**: Other emulators are reference material, not ground truth. Always attribute emulator-sourced findings explicitly ("SameBoy does X", "Stella does X") rather than as hardware fact. The traced reference emulators are all behavioural — they corroborate a finding but never ground it.
- **Data-driven debugging**: Use available data resources (each core's netlist/sim, hardware docs, timing measurements, cross-emulator traces — enumerated in its methodology doc) rather than reasoning about behavior from code alone. Observe first, hypothesize second.
- **A green suite is a floor, not a ceiling**: A full pass means no known divergence, not no divergence. Corpus completeness is a claim about the corpus, not the model.
- **Regressions from structurally-motivated change are information**: They mark where the old model was fitted — record and understand them rather than reflexively reverting the structure. The commit gate is unchanged: the suite is green before merge.
- **Flag gaps, don't paper over them**: When a core's primary ground-truth source doesn't cover a behaviour but a measurement could provide it (a dmg-sim run on DMG, a Sim2600 run on VCS), raise it with the user rather than falling back to emulator source or hand-waving. Where a core has no gate-level oracle at all (CGB; the VCS RIOT), escalate to the user instead of substituting an emulator's behaviour for hardware fact.
- **Future cores**: hardware-first applies to every core, but the internal timing mechanism is per-core — use each system's available evidence to reach the highest verifiable accuracy (gate-level lockstep where a die sim exists; coarser quanta where test ROMs are the ceiling), chosen in that core's methodology doc. The invariant contract every core owes the app/debugger/tests is listed in `docs/adding-a-system.md`.

## Per-core methodology

The shared skill-system rules above apply to every core. Each core's **ground-truth hierarchy, resources, and timing model** live beside its crate and load on demand — read the relevant one before any accuracy work on that core, and don't carry another core's resource detail into an unrelated session:

| Core | Crate | Methodology doc |
|------|-------|-----------------|
| Game Boy (DMG) | `missingno-gb` | `crates/systems/gb/AGENTS.md` — the shared-silicon base: DMG ground-truth hierarchy, clock model, instruction execution, core internals |
| Game Boy Color | `missingno-gbc` | `crates/systems/gbc/AGENTS.md` — no gate-level sim; hardware test-ROM values lead |
| Atari VCS | `missingno-vcs` | `crates/systems/vcs/AGENTS.md` — Sim2600 (CPU+TIA) + datasheet/schematics (RIOT); behavioural VCS emulators last |
| Sega SG-1000 | `missingno-sg1000` | `crates/systems/sg1000/AGENTS.md` — a board over zilog-z80 + ti-vdp + ti-psg; Enri's traced schematics lead, Soggy corroborates, MAME last |
| Sega Master System | `missingno-sms` | `crates/systems/sms/AGENTS.md` — a first-pass core; its oracle ceiling is its committed console tests |
| Nintendo NES | `missingno-nes` | `crates/systems/nes/AGENTS.md` — a first-pass core; its oracle ceiling is its committed console tests |

CGB is a superset of DMG, so its doc builds on the DMG base; the VCS core shares no silicon with the Game Boy and stands alone. Adding a core = one row here plus an `AGENTS.md` beside its crate. Shared chip crates carry the same kind of doc (`crates/chips/<chip>/AGENTS.md` — the chip's own conformance oracle and references); in-system, the console's doc outranks the chip's. The crate docs auto-load when you work in that subtree.

## Investigation hygiene

- **DMG spec-gap workflow lives in the GB core doc.** The spec-gap discipline (surface dmg-sim measurement targets to the user; check `receipts/ppu-overhaul/spec-gaps/` before claiming a spec defect; grep failing test names across spec-gaps and prior investigations first) is DMG-specific — see `crates/systems/gb/AGENTS.md`. The general receipt/framing discipline below applies to every core.
- **Subagent receipts are starting points, not load-bearing claims.** Receipts compress information — they can elide qualifiers, miss adjacent paragraphs that pre-empt the apparent finding, or summarise a measurement without flagging that it was at the wrong sub-phase. When a downstream decision (open a spec-gap, refute a hypothesis, claim a contradiction, declare an investigation resolved) rides on a specific cited line — spec section, code path, FST timestamp, trace entry index — open the source and read it directly before acting.
- **Prior investigation receipts are background, not authority.** The `receipts/investigations/` archive is a record of past attempts and decisions. Constants, file:line citations, and structural claims in old receipts go stale fast — missingno's PPU/CPU subsystems are refactored frequently. Use prior receipts to understand *what was tried and why*, not to import specific facts. Verify any concrete code claim against the current source before relying on it. When briefing a `/research` subagent, frame prior receipts as "background, may be stale" and ask for current-code verification rather than citation.
- **Subagent reports contain framings, not just facts.** Subagent receipts mix two kinds of content: measurements (numeric values, file:line citations, verbatim quotes) and framings (visual impressions, "this means", "the pattern suggests N-pixel shift"). Framings are subagent interpretations, not measurements — they can be wrong even when the underlying observations are right. Before building hypotheses on a framing, demote it to "subagent interpretation, unverified" and check the underlying measurement directly. The cost of a 30-second direct check is much smaller than a chain of dispatches built on a wrong framing.
- **Current-state claims in a brief are hints, not facts.** A dispatch brief that quotes a failure signature, mismatch geometry, or test count describes the tree at dispatch time — and the tree moves during long sessions (fixes land on main while agents run). Fact-finding subagents should re-verify any caller-supplied claim about the current tree (re-run the failing test, re-extract the mismatch positions) before building measurements or conclusions on it, and report the fresh value alongside anything that changed.
- **Verify items before user-attention lists.** When writing "for you to verify" or "open questions" lists for the user, verify each item locally first. The user's attention is the constrained resource — items in those lists should be ones genuinely uncheckable from the dispatcher's side (e.g., requires hardware, requires running dmg-sim, requires deep domain expertise). If a claim ends up self-resolving when checked, drop it from the list rather than keeping it as "open".

## Code Style (committed code in `crates/`)

These rules apply to all committed Rust code — production code, tests, and doc-comments. They override generic "explain everything" instincts.

- **Comments are sparse.** Default to no comment. Add one only when WHY is non-obvious — a hidden constraint, a subtle invariant, a workaround. Don't explain WHAT the code does; well-named identifiers do that. One short line max; never multi-paragraph docstrings or multi-line block comments.
- **Comments are untested claims.** The suites verify code, never prose — a wrong or overstated comment survives every gate. Any comment stating a hardware fact must be checkable against the spec/concordance (or the cited CGB reference) and phrased no stronger than that grounding supports ("captures at the M-boundary fall" only if the spec says so; "appears to"/"tuned against test X" belongs in receipts, not comments). When in doubt, shorten or delete.
- **Identifiers are semantic; gate names live in comments.** Fields, methods, locals, and types are named for their ROLE (`line_end_pending`, `crossed_window_registers`) — never for the gate (`nype`, `rydy`). The gate name ties the identifier to the silicon in the adjacent one-line comment, verified against the spec's concordance. (The spec leads with gate names; the emulator leads with role names — don't import the spec's direction into `crates/`.)
- **Reference hardware via gate names, not spec § numbers.** When a comment ties code to hardware, name the gate (`NYXU`, `RYDY`, `CATU`) and what it does in one phrase. **Never write `§6.12` or `spec §X.Y` in committed code** — the spec gets renumbered, the concordance moves, and stale section refs rot. Spec section numbers belong in the spec, in the gate concordance, and in receipts — not in `crates/`.
- **No narration of verification outcomes.** Don't write comments like "this matches SameBoy", "fixes test X", "per the April 24 investigation". Those belong in commit messages and PR descriptions, which decay with code less destructively than rotting comments do.
- **No `// added for …` / `// removed because …` provenance comments.** Git blame answers those questions.
- **When invoking `/design` or `/implement`**, these skills run in-context on the main agent — meaning your conversation memory IS the brief, and the rules above need to actively bind your output. Do not paste spec-section references into the brief — if you write `§6.12` while in design/implement mode, you will paste it into doc-comments. Re-read this Code Style section when entering implement mode if you've drifted.

## Architecture

The project is a Cargo workspace with crates under `crates/`:

- **`crates/missingno/core/`** (`missingno-core`) — The system-agnostic foundation every core and every client of the stack shares. No console silicon of its own; it holds the shared *vocabulary* and the *seam*: the behavioural seam traits (`SystemConsole`, `SystemDebugger`, and the `Machine` hook trait behind them) plus the plain data they exchange (`system.rs`); the board/TV/analog hardware vocabulary (`tv.rs`, `analog.rs`, `video.rs`'s `DisplayTechnology`); the debugger presentation and state vocabularies (`inspect.rs` sections, `graphics.rs`, `waveform.rs`, `disasm.rs`/`isa.rs`, `symbols.rs`, `cdl.rs`); the hardware-named `SystemStateSchema` (`state.rs`); and the three state containers keyed on it — save states (`state_file.rs`, `MPSV`), input recordings with deterministic replay (`recording.rs`, `MPRC`), the trace container being `MPRK`. A core *states* its hardware here; applying any of it is the app's job.
- **`crates/systems/gb/`** (`missingno-gb`) — Shared-silicon emulation library + the DMG model. No GUI dependencies. Contains:
  - **`crates/systems/gb/src/`** — The generic core: `Console<M: Model>` holds the shared hardware in a `Chassis<M>` struct (`chassis.cpu`, `chassis.ppu`, `chassis.audio`, `chassis.timers`, `chassis.clock`, …) plus the per-console `model: M`. The `Model` / `PpuModel` / `ApuSpec` traits are the DMG↔CGB divergence seam: associated types for console-specific state, associated consts for static silicon properties, hooks (some taking `&mut Chassis`) for console-specific behaviour. `GameBoy = Console<Dmg>`. `Console::step()` executes one instruction and returns a `StepResult` with `new_screen`, `sram_dirty`, and `tcycles`.
  - **`crates/systems/gb/src/debugger/`** — Debugging backend. Generic `Debugger<M>` wrapping `Console<M>` with breakpoints, stepping, disassembly, and a T-cycle counter.
  - **`crates/systems/gb/tests/accuracy/`** — Integration tests (ROM-based accuracy tests).
- **`crates/systems/gbc/`** (`missingno-gbc`) — The CGB model: `GameBoyColor = Console<Cgb>`. Owns everything CGB-specific — colour PPU (`CgbPpu`, CRAM, VRAM banking), `CgbApu`, the HDMA/GDMA engine, the KEY1 speed-switch blackout, DMG-compat palettes — attached through the `Model` seam. The DMG build provably contains none of this (its monomorphization has no CGB-mechanism symbols).
- **`crates/systems/vcs/`** (`missingno-vcs`) — The Atari VCS core: a 6507 over `missingno-mos-6502`, the TIA, the RIOT, and the cartridge board set, with its own debugger backend rather than the stepping shortcut.
- **`crates/systems/sg1000/`** (`missingno-sg1000`) — The Sega SG-1000 board, composed from `missingno-zilog-z80`, `missingno-ti-vdp` and `missingno-ti-psg` at the board's crystal.
- **`crates/systems/sms/`** (`missingno-sms`) and **`crates/systems/nes/`** (`missingno-nes`) — First-pass cores, outside the workspace default members (build and test them with `-p`). Read them for facts about their hardware, never as an exemplar to copy.
- **`crates/chips/`** — Shared chip crates, maker-prefixed: `missingno-mos-6502` (VCS, NES), `missingno-zilog-z80` (SG-1000, SMS), `missingno-ti-vdp` and `missingno-ti-psg` (SG-1000; the PSG also serves the SMS). Each carries its own methodology doc.
- **`crates/missingno/app/`** (`missingno`) — Iced 0.14 GUI binary (`crates/missingno/app/src/app/`). **The app shell is system-agnostic** (drives consoles through the `missingno-core` seam traits; per-family registration is the `FAMILIES` descriptor table — never parallel Dmg/Cgb dispatch enums) and **the app is a session client, not a machine owner** (it hosts a `missingno-session` `SharedSession`; the session thread owns the console for its whole life). App detail — app-shell seams, session bridging, the UI-automation surface, the debugger pane system, sidecars, off-chip peripherals, and the app verification recipe — is in `crates/missingno/app/AGENTS.md` (auto-loads when working in that subtree).
- **`crates/missingno/session/`** (`missingno-session`) — The session component: one owner of an emulated machine, hosted on its own thread, that every consumer drives as a client. `SharedSession` holds the machine (a debugger-hosting `Session` or a plain console), runs the paced free-run loop, publishes the latest-value slots, fans out `SessionEvent`s, and serializes every client's commands through one queue; `SessionHandle` is the cloneable client. Save states, recording capture, watchable playback and checkpoint-verified replay are all session commands, so there is one implementation of each. The `factory.rs` registry is the one point that knows concrete cores (a media predicate plus a constructor per core, feature-gated, with generic `LoadOptions` a core may honour — the VCS broadcast standard, a Game Boy boot ROM). It links no transport: an embedder depending on this crate alone gets a running, inspectable machine. The `tools` feature adds the session's own agent tool surface (`tools.rs` — the MCP tool vocabulary, no transport) and the Unix-socket `attach` endpoint and client that publish it to another process.
- **`crates/missingno/debugger/`** (`missingno-debugger`) — The servers that publish a session. Two transports, both clients of `missingno-session`: HTTP (`missingno-debugger [<rom>] [--port N] [--boot-rom PATH]`, default port 3333) for scripted/bulk access, and MCP-over-stdio (`--mcp`) for interactive agent use. The transports carry no core-specific code, so a core registered in the session factory gets the whole server. `--allow-attach` publishes this process's own session for other clients. With `--mcp` and no ROM the MCP server starts **idle**, advertising only `load_rom`/`attach`/`eject`/`status` and gaining the session's full tool set once `load_rom` recognises a ROM or `attach` reaches a session another process published — one static server entry (`.mcp.json`, at repo root) serves any ROM and any host. The `dbg_*` shell helpers (`scripts/debugger.sh`) wrap the HTTP routes.
- **`crates/missingno/remote/`** (`missingno-remote`) — The MCP-over-stdio server for the GUI's UI-automation surface; the app-side twin of `missingno-debugger --mcp`. Starts idle advertising `attach`/`detach`/`status`, discovers `ui-*.sock` publications, and forwards `tools/list`/`tools/call` verbatim once attached, so new app-side tools appear without server changes. No internal-crate dependencies (the socket client is deliberately duplicated); registered in `.mcp.json`.
- **`crates/missingno/iced/`** (`missingno-iced`) — Shared iced presentation for both GUI binaries: the emulator screen widget and its device-simulation shader (`ScreenView`, `PalettePolicy`, the frame types), the paddle wind widget, and the wgpu texture pipeline behind them.
- **`crates/missingno/curator/`** (`missingno-curator`) — A separate iced binary for reviewing, enriching and confirming `missingno-gamedb` entries. It hosts a real `missingno-session` for playtesting and publishes its own `ui-*.sock` endpoint, so `missingno-remote` forwards its tools with no server change.

Each core's internals and resources live beside its crate (see *Per-core methodology*): the DMG/Game Boy core detail — instruction execution, the clock/phase model, the `Console<M>`/`Chassis` composition and core patterns, and the DMG data-source hierarchy — is in `crates/systems/gb/AGENTS.md`.

- **Config**: `settings.ron` and `recent.ron` in platform config dir via `dirs` crate. Uses `jiff` (not `chrono`) for timestamps.

### Debugger

- **Adding an emulated system**: read `docs/adding-a-system.md` — the seam map (core `Model` axis vs the app's `app/system/` family axis) and the honest inventory of remaining GB-shaped surfaces.
- **App debugger detail lives in `crates/missingno/app/AGENTS.md`**: the pane system and its app-side family registry, the app verification recipe, debug sidecars (`.sym`/`.cdl`), and off-chip peripherals (printer, RTC saves, softpatching).
- **State, traces, and recordings**: one `SystemStateSchema` per core (`missingno-core`'s `state.rs`) drives save states, trace capture and input recordings off the same field vocabulary — `docs/adding-a-system.md` owns the containers and what wiring each costs.

### Resources

External resources live in `receipts/resources/` (gitignored — clone or download what you need). **Core-specific resources are enumerated in each core's methodology doc**: DMG's dmg-sim / DMG Timing Spec / gb-ctr / propagation-delay-analysis / gb-timing-data / slowpeek in `crates/systems/gb/AGENTS.md`; VCS's Sim2600 / Stella Programmer's Guide / TIA_HW_Notes / 6532 datasheet in `crates/systems/vcs/AGENTS.md`. The one tool every core shares:

| Directory | Repository | Description |
|-----------|------------|-------------|
| `morepork` | https://github.com/ajoneil/morepork | Execution trace capture, diff, and render across emulators and systems (DMG/CGB/VCS). In-tree dev tooling: the `morepork` cargo feature pulls the crate as a git dependency (local iteration lands by pushing to the repo and `cargo update -p morepork`); its `MPRK` container is re-founded on the shared `missingno-core` state vocabulary, so trace columns are schema fields. |
