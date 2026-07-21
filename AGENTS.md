# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Skill System Rules

These are the top-level rules governing how skills interact. They survive context compaction and override any default agent behavior.

1. **Always use skills — never ad-hoc.** When asked to investigate, debug, research, or analyze, invoke the appropriate skill (`/investigate`, `/research`, `/compare-traces`, etc.). Never start ad-hoc analysis, use WebSearch directly, read reference emulator source yourself, or trace behavior in your head. The skill system exists to enforce scope discipline and produce durable receipts. Bypassing it produces unreliable, unreproducible results that don't survive context compaction.

2. **Hardware is the source of truth.** The goal is always to understand what the real hardware does and model that behavior. Prioritize hardware documentation, decaps, test measurements, and direct hardware observations over any emulator implementation. Other emulators (SameBoy, Gambatte, DocBoy, etc.) are reference material — useful for confirming hardware behavior, but never the primary source and never a model to copy. The question is always "what does the hardware do?" not "what does emulator X do?" Each core's ground-truth hierarchy differs and lives beside its crate (see *Per-core methodology*) — read the core's doc before any accuracy work there. The rule holds everywhere, but what operationalises it (gate-level sim, hardware captures, test-ROM values) is per-core, and CGB and the VCS RIOT have no gate-level source at all.

3. **Skills are subroutine calls — never stopping points.** When a subagent skill returns, or an in-context skill exits, the caller MUST immediately read the receipt, update `summary.md`, and continue the investigation in the same turn. Never end your turn after a skill produces its receipt. Skill invocations are function calls, not async tasks you wait on.

4. **summary.md is the single source of truth for investigation state.** Update it before every skill dispatch and after every skill return — no exceptions. If context were compacted right now, `summary.md` alone must tell you exactly where you are and what to do next. summary.md is owned by the `/investigate` dispatcher, not by skills — when an in-context skill exits, you exit its mode first and then update summary.md as the dispatcher.

5. **Use available data before generating new data.** Before instrumenting code or running the debugger, check whether the question can be answered with existing resources. The **ordered reference hierarchy is per-core** — it lives in that core's methodology doc (`crates/missingno-<core>/AGENTS.md`; see *Per-core methodology* below) because each core's ground truth differs. Read the target core's hierarchy first, and also check **existing research** (`receipts/research/`) and prior investigations. Generate new diagnostic data only when these existing sources don't answer the question.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored. **Never reference receipt paths in committed code** (comments, commit messages, etc.) — they are ephemeral working documents, not permanent artifacts.
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

Missingno is a Rust emulator and debugger. Its mature core is the Game Boy family (DMG + Game Boy Color); additional console cores (Atari VCS, and early SMS and NES) share the frontend, debugger, and skill infrastructure. Each core's accuracy methodology lives beside its crate (see *Per-core methodology*).

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

- Run tests against the core you're working on: `cargo test -p missingno-gb` (DMG), `-p missingno-gbc` (CGB), or `-p missingno-vcs` (Atari VCS). Do not run `cargo test` against the whole workspace unless specifically asked.
- **The GB, GBC, and VCS suites all fully pass** — the gate for any change is a fully-passing suite; ANY failure is a regression. The VCS references are hardware-endorsed (see its methodology doc), so a VCS screenshot regression is a divergence from measured silicon. The report scripts exist for accuracy investigations and regression triage; for ordinary changes against the green baseline, plain `cargo test` under `timeout` suffices.
- For regression checking, use `./scripts/test-report-gb.sh --diff` instead of raw `cargo test`. It generates structured reports with baseline comparison and saves them to `receipts/test-reports/gb/`. Variants: `./scripts/test-report-gbc.sh` (reports under `receipts/test-reports/gbc/`) and `./scripts/test-report-vcs.sh` (`receipts/test-reports/vcs/`); use the one matching the core you're changing.
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
- **Future cores**: hardware-first applies to every core, but the internal timing mechanism is per-core — use each system's available evidence to reach the highest verifiable accuracy (gate-level lockstep where a die sim exists; coarser quanta where test ROMs are the ceiling), chosen in that core's methodology doc. The invariant contract every core owes the frontend/debugger/tests is listed in `docs/adding-a-system.md`.

## Per-core methodology

The shared skill-system rules above apply to every core. Each core's **ground-truth hierarchy, resources, and timing model** live beside its crate and load on demand — read the relevant one before any accuracy work on that core, and don't carry another core's resource detail into an unrelated session:

| Core | Crate | Methodology doc |
|------|-------|-----------------|
| Game Boy (DMG) | `missingno-gb` | `crates/missingno-gb/AGENTS.md` — the shared-silicon base: DMG ground-truth hierarchy, clock model, instruction execution, core internals |
| Game Boy Color | `missingno-gbc` | `crates/missingno-gbc/AGENTS.md` — no gate-level sim; hardware test-ROM values lead |
| Atari VCS | `missingno-vcs` | `crates/missingno-vcs/AGENTS.md` — Sim2600 (CPU+TIA) + datasheet/schematics (RIOT); behavioural VCS emulators last |

CGB is a superset of DMG, so its doc builds on the DMG base; the VCS core shares no silicon with the Game Boy and stands alone. Adding a core = one row here plus an `AGENTS.md` beside its crate. The crate docs auto-load when you work in that subtree.

## Investigation hygiene

- **DMG spec-gap workflow lives in the GB core doc.** The spec-gap discipline (surface dmg-sim measurement targets to the user; check `receipts/ppu-overhaul/spec-gaps/` before claiming a spec defect; grep failing test names across spec-gaps and prior investigations first) is DMG-specific — see `crates/missingno-gb/AGENTS.md`. The general receipt/framing discipline below applies to every core.
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

- **`crates/missingno-core/`** (`missingno-core`) — The system-agnostic foundation every core and both frontends share. No console silicon of its own; it holds the shared *vocabulary* and the *seam*: the behavioural seam traits (`SystemConsole`, `SystemDebugger`, and the `SteppingSystem` shortcut) plus the plain data they exchange (`system.rs`); the board/TV/analog hardware vocabulary (`tv.rs`, `analog.rs`, `video.rs`'s `DisplayTechnology`); the debugger presentation and state vocabularies (`inspect.rs` sections, `graphics.rs`, `waveform.rs`, `disasm.rs`/`isa.rs`, `symbols.rs`, `cdl.rs`); the hardware-named `SystemStateSchema` (`state.rs`); and the three state containers keyed on it — save states (`state_file.rs`, `MPSV`), input recordings with deterministic replay (`recording.rs`, `MPRC`), the trace container being `MPRK`. A core *states* its hardware here; applying any of it is the frontend's job.
- **`crates/missingno-gb/`** (`missingno-gb`) — Shared-silicon emulation library + the DMG model. No GUI dependencies. Contains:
  - **`crates/missingno-gb/src/`** — The generic core: `Console<M: Model>` holds the shared hardware in a `Chassis<M>` struct (`chassis.cpu`, `chassis.ppu`, `chassis.audio`, `chassis.timers`, `chassis.clock`, …) plus the per-console `model: M`. The `Model` / `PpuModel` / `ApuSpec` traits are the DMG↔CGB divergence seam: associated types for console-specific state, associated consts for static silicon properties, hooks (some taking `&mut Chassis`) for console-specific behaviour. `GameBoy = Console<Dmg>`. `Console::step()` executes one instruction and returns a `StepResult` with `new_screen`, `sram_dirty`, and `tcycles`.
  - **`crates/missingno-gb/src/debugger/`** — Debugging backend. Generic `Debugger<M>` wrapping `Console<M>` with breakpoints, stepping, disassembly, and a T-cycle counter.
  - **`crates/missingno-gb/tests/accuracy/`** — Integration tests (ROM-based accuracy tests).
- **`crates/missingno-gbc/`** (`missingno-gbc`) — The CGB model: `GameBoyColor = Console<Cgb>`. Owns everything CGB-specific — colour PPU (`CgbPpu`, CRAM, VRAM banking), `CgbApu`, the HDMA/GDMA engine, the KEY1 speed-switch blackout, DMG-compat palettes — attached through the `Model` seam. The DMG build provably contains none of this (its monomorphization has no CGB-mechanism symbols).
- **`crates/missingno/`** (`missingno`) — Iced 0.14 GUI binary. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer. Lives in `crates/missingno/src/app/`. **The app shell is system-agnostic**: it drives consoles through the object-safe seam traits (`SystemConsole` for the plain emulator, `SystemDebugger` for stepping/breakpoints/inspection) defined in `missingno-core` and re-exported through `app/system/`. The Game Boy family implements both once, generically over its `Model`, as `GbConsole<M>` in `missingno-gb`'s `system.rs` (reused unchanged by the headless factory); simple stepping cores (NES, SMS) get the seam from `missingno-core`'s `SteppingConsole`/`SteppingDebugger`. The GUI's per-family registration is the `FAMILIES` descriptor table in `app/system/mod.rs` — one `FamilyDescriptor` per family carrying its `Platform` identity, extensions, control labels, `is_rom` predicate, `title_from_rom` hook, `create_console` factory (CGB-aware header ⇒ CGB core), and optional `trace` entry point; `family_for` is the single classification point. Adding a system family = a new `app/system/` submodule + a `FAMILIES` row — never parallel Dmg/Cgb dispatch enums (don't reintroduce them). **The app is a session client, not a machine owner**: it hosts a `missingno-session` `SharedSession` and drives it through a cloneable `SessionHandle` (`app/emulation.rs`, `app/session_bridge.rs`). The session thread owns the console for its whole life — run/pause is a command, never an ownership handoff — and publishes latest-value slots (frame, running status, and, for a debugger session, a per-vblank `DebugView` snapshot and the memory-interest windows) plus a `SessionEvent` stream the bridge feeds into the iced subscription. Paused deep inspection is one cached owned readout fetched through the handle. The app links no server: with the session's `tools` feature it publishes an attach socket when the external-clients setting is on, and the HTTP/stdio servers stay in `missingno-debugger`. The app also owns a **UI-automation surface** (`app/automation/`): a semantic UI tree (stable dot-namespaced element ids + labels from `registry.rs`, live bounds via a widget `Operation`), message-injection activation, `set_text`/`scroll_to`, window resize, and window screenshots (`iced::window::screenshot`, cropping to an element or region, lifting the min-size floor for small captures). It is published as newline-JSON-RPC over `ui-<pid>.sock` in the same runtime dir as the attach socket, gated by the `allow_ui_automation` setting or the `--allow-ui-automation` flag. Registry labels are written as accessible names — they become the AccessKit inventory when iced ships accessibility (planned for 0.15; 0.14 has none). Client-initiated resizes are dropped on GNOME Wayland (winit limitation) — launch under XWayland (`WAYLAND_DISPLAY= missingno`) for exact-size captures; the tools report the miss when it happens.
- **`crates/missingno-session/`** (`missingno-session`) — The session component: one owner of an emulated machine, hosted on its own thread, that every consumer drives as a client. `SharedSession` holds the machine (a debugger-hosting `Session` or a plain console), runs the paced free-run loop, publishes the latest-value slots, fans out `SessionEvent`s, and serializes every client's commands through one queue; `SessionHandle` is the cloneable client. Save states, recording capture, watchable playback and checkpoint-verified replay are all session commands, so there is one implementation of each. The `factory.rs` registry is the one point that knows concrete cores (a media predicate plus a constructor per core, feature-gated, with generic `LoadOptions` a core may honour — the VCS broadcast standard, a Game Boy boot ROM). It links no transport: an embedder depending on this crate alone gets a running, inspectable machine. The `tools` feature adds the session's own agent tool surface (`tools.rs` — the MCP tool vocabulary, no transport) and the Unix-socket `attach` endpoint and client that publish it to another process.
- **`crates/missingno-debugger/`** (`missingno-debugger`) — The servers that publish a session. Two transports, both clients of `missingno-session`: HTTP (`missingno-debugger [<rom>] [--port N] [--boot-rom PATH]`, default port 3333) for scripted/bulk access, and MCP-over-stdio (`--mcp`) for interactive agent use. The transports carry no core-specific code, so a core registered in the session factory gets the whole server. `--allow-attach` publishes this process's own session for other clients. With `--mcp` and no ROM the MCP server starts **idle**, advertising only `load_rom`/`attach`/`eject`/`status` and gaining the session's full tool set once `load_rom` recognises a ROM or `attach` reaches a session another process published — one static server entry (`.mcp.json`, at repo root) serves any ROM and any host. The `dbg_*` shell helpers (`scripts/debugger.sh`) wrap the HTTP routes.

Each core's internals and resources live beside its crate (see *Per-core methodology*): the DMG/Game Boy core detail — instruction execution, the clock/phase model, the `Console<M>`/`Chassis` composition and core patterns, and the DMG data-source hierarchy — is in `crates/missingno-gb/AGENTS.md`.

- **Config**: `settings.ron` and `recent.ron` in platform config dir via `dirs` crate. Uses `jiff` (not `chrono`) for timestamps.

### Debugger

- **Adding an emulated system**: read `docs/adding-a-system.md` — the seam map (core `Model` axis vs frontend `app/system/` family axis) and the honest inventory of remaining GB-shaped surfaces.
- **Pane system**: `crates/missingno/src/app/debugger/panes.rs` holds a `pane_grid` of `Box<dyn Pane>` trait objects. Each pane is a separate module owning its struct (e.g. `ScreenPane`), its optional `Message` enum, and its `impl Pane` (render via `view(Option<&PaneContext>)` — `Some` = live console or snapshot through the `InspectSource` trait, `None` = "Running…" placeholder). **Registering a pane = one registry entry** (kind, icon, label, constructor, `instanceable`) plus a `DebuggerPane` variant; nothing else to thread through. Each family provides its own registry through `panes::Family` (`pane_family()` is a required seam method; `PANE_FAMILIES` lists them) — a family registers only the panes its core feeds. A pane is either single-instance (rail click toggles) or instanceable (the Memory pane opens a fresh instance per click, and a per-instance message leaves siblings untouched). **Panes render exclusively from the typed surfaces the shared `PaneContext` carries — there is no family-specific pane escape hatch**, so a family reaches the grid only by filling the seam surfaces the generic panes read, and surfaces its own chip state through the sidebar `Section`s instead. New pane data must be reachable from the seam's inspection surfaces (the live console while paused, the per-vblank `InspectSnapshot` while running), so extend both when adding inspection state. Pane layout persists to `debugger_layout.ron` keyed by registry labels (unknown label ⇒ whole saved layout discarded).
- **Frontend verification recipe**: `cargo check` + `cargo clippy` (no NEW warnings) + `cargo test -p missingno` after every stage; GUI behaviour changes need a manual run (`cargo run -- <rom> --debugger`) since panes/layout aren't machine-verifiable. iced pitfalls: `pane_grid` view closures run eagerly (short-lived captures are fine); widget `Element`s should copy data eagerly rather than borrow inspection state; `iced::Task` messages route through the single `update()` — keep one sink per subsystem (library messages all route through `library::update::handle`).
- **Debug sidecars**: the core debugger auto-loads `<rom>.sym` labels (no$gmb/RGBDS format, `missingno-core`'s `symbols.rs`) and a `<rom>.cdl` code/data log (`cdl.rs` there too, Mesen-compatible flags + a missingno instruction-start bit), and saves both on close. Banked labels resolve through each mapper's `switchable_rom_bank()`. The disassembly shows label rows, bank-prefixed addresses, `db` rows for logged data bytes, and exact backward context where the log has coverage. Users create/edit labels in the Labels bottom panel (written back append-only under a `; missingno user labels` section). Watchpoints and breakpoints each have a bottom panel; a CGB game's extra chip state (speed/SVBK in the CPU section, VBK/OPRI/BCPS/OCPS/HDMA in the PPU section) arrives as extra rows in the sections it belongs to, not a section of its own.
- **Off-chip peripherals**: the Game Boy Printer is a frontend `SerialLink` (`crates/missingno/src/printer.rs`), auto-attached when no explicit link is configured; prints save as PNGs under `prints/` in the game's library folder. Battery saves append the standard RTC tail on clock carts (`crates/missingno/src/sram.rs`); the core stays wall-clock-free (`Cartridge::rtc`/`restore_rtc`). ROMs softpatch from `.ips`/`.bps` beside them (`crates/missingno/src/patch.rs`).
- **State, traces, and recordings**: keyed on a core's `SystemStateSchema` (`missingno-core`'s `state.rs`), one schema drives save states (`state_file.rs`, `MPSV`), schema-driven trace capture (the `MPRK` container; `crates/missingno-gb/src/trace.rs` is the worked bridge), and input recordings (`recording.rs`, `MPRC` — an initial save state plus a frame-indexed input trace). Recording and deterministic replay ride entirely on the existing seam (`save_state`/`load_state`/`set_control`/`step_frame`), so a core that wires save states gets replay for free; the session's replay and `missingno-core`'s `recording::replay` verify frame-hash checkpoints and name the frame a divergence first appears on.

### Resources

External resources live in `receipts/resources/` (gitignored — clone or download what you need). **Core-specific resources are enumerated in each core's methodology doc**: DMG's dmg-sim / DMG Timing Spec / gb-ctr / propagation-delay-analysis / gb-timing-data / slowpeek in `crates/missingno-gb/AGENTS.md`; VCS's Sim2600 / Stella Programmer's Guide / TIA_HW_Notes / 6532 datasheet in `crates/missingno-vcs/AGENTS.md`. The one tool every core shares:

| Directory | Repository | Description |
|-----------|------------|-------------|
| `morepork` | https://github.com/ajoneil/morepork | Execution trace capture, diff, and render across emulators and systems (DMG/CGB/VCS). In-tree dev tooling: the `morepork` cargo feature path-depends on this clone (`receipts/resources/morepork/crates/morepork`), so enabling capture requires it present; its `MPRK` container is re-founded on the shared `missingno-core` state vocabulary, so trace columns are schema fields. |
