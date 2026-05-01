# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Skill System Rules

These are the top-level rules governing how skills interact. They survive context compaction and override any default agent behavior.

1. **Always use skills — never ad-hoc.** When asked to investigate, debug, research, or analyze, invoke the appropriate skill (`/investigate`, `/research`, `/compare-traces`, etc.). Never start ad-hoc analysis, use WebSearch directly, read reference emulator source yourself, or trace behavior in your head. The skill system exists to enforce scope discipline and produce durable receipts. Bypassing it produces unreliable, unreproducible results that don't survive context compaction.

2. **Hardware is the source of truth.** The goal is always to understand what the real hardware does and model that behavior. Prioritize hardware documentation, decaps, test measurements, and direct hardware observations over any emulator implementation. Other emulators (SameBoy, Gambatte, GateBoy, etc.) are reference material — useful for confirming hardware behavior, but never the primary source and never a model to copy. The question is always "what does the hardware do?" not "what does emulator X do?"

3. **Skills are subroutine calls — never stopping points.** When a subagent skill returns, or an in-context skill exits, the caller MUST immediately read the receipt, update `summary.md`, and continue the investigation in the same turn. Never end your turn after a skill produces its receipt. Skill invocations are function calls, not async tasks you wait on.

4. **summary.md is the single source of truth for investigation state.** Update it before every skill dispatch and after every skill return — no exceptions. If context were compacted right now, `summary.md` alone must tell you exactly where you are and what to do next. summary.md is owned by the `/investigate` dispatcher, not by skills — when an in-context skill exits, you exit its mode first and then update summary.md as the dispatcher.

5. **Use available data before generating new data.** Before instrumenting code or running the debugger, check whether the question can be answered with existing resources. Primary references in order:
   1. **PPU timing model spec** (`receipts/ppu-overhaul/reference/ppu-timing-model-spec.md`) — gate-level hardware behaviour collated from dmg-sim measurements, netlist analysis, and propagation-delay analysis. This is the canonical hardware reference for the PPU. If the spec doesn't answer the question but a dmg-sim run would, **flag the gap to the user** so the spec can be updated — do not silently substitute emulator source or fall back to lower-priority references.
   2. **gekkio's gb-ctr** (Game Boy Complete Technical Reference, https://gekkio.fi/files/gb-docs/gbctr.pdf) — detailed, reliable hardware reference covering the whole console.
   3. **dmg-sim** (`receipts/resources/dmg-sim/`) — gate-level SystemVerilog simulation derived from the DMG-CPU B netlist. Use `scripts/dmg-sim-observe.sh` to run a ROM and capture an FST waveform for signal observation when the spec lacks the needed detail.
   4. **Propagation delay analysis** (`receipts/resources/gb-propagation-delay-analysis/`), **existing research** (`receipts/research/`), and **cross-emulator traces** (gbtrace). For trace comparisons, **prefer the GateBoy adapter** — it is die-photo-derived and closest to hardware — and fall back to Gambatte/SameBoy/DocBoy only when GateBoy does not pass the test.

   Generate new diagnostic data only when these existing sources don't answer the question.

## Agent Infrastructure

- **`AGENTS.md`** — Canonical agent instructions. Tool-specific config files (e.g. `CLAUDE.md`) symlink here so all agents share a single source of truth.
- **`.agents/skills/`** — Canonical skill/command definitions (slash commands). Tool-specific command directories (e.g. `.claude/commands/`) symlink here. **Symlinks between these directories are user-managed. Do not modify them.**
- **`receipts/`** — Output directory for skill executions. Skills should write any persistent output (logs, reports, diffs) here. Gitignored. **Never reference receipt paths in committed code** (comments, commit messages, etc.) — they are ephemeral working documents, not permanent artifacts.
- **`receipts/resources/`** — External resources: sibling projects, reference emulator source, hardware schematics, etc. Clone or download whatever you need into this directory. It's gitignored, so treat it as a workspace for external material.

### Context hygiene

The conversation context is volatile — it will be compacted unpredictably. Treat files on disk as the primary memory; conversation is scratch space.

- **Write early, write often.** After every meaningful step, write it to the appropriate file. Test: if context were compacted right now, would your progress survive?
- **Keep context lean.** After writing state to disk, re-read the file when you need the information — don't carry it in conversation memory.
- **After context compaction**: Re-read the active skill file from `.agents/skills/` and `summary.md` before continuing. The compaction summary won't preserve skill directives.
- **Periodically during long sessions**: Every ~10 tool calls, re-read the active skill file and `summary.md` to catch drift.

### Skill invocation protocol

Skills invoke other skills as subroutines. There are two execution flavors:

**Subagent skills** — `/research`, `/analyze`, `/instrument`, `/inspect`, `/compare-traces`. These are fact-finding tasks that produce large diagnostic outputs (file reads, source exploration, measurement data, test output). They run as Task subagents (`subagent_type: "general-purpose"`) so that intermediate work stays out of the main context window. Each subagent receives its skill file in the Task prompt, writes a receipt file, and stops. It does NOT inherit the caller's context or hypotheses.

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

Missingno is a Game Boy emulator and debugger written in Rust.

## Build and Run Commands

**Do not use `--release` unless explicitly asked.** Release builds are slow to compile. Debug builds are the default for development, testing, and debugging.

```bash
cargo run                                    # Build and run (debug)
cargo run -- path/to/rom.gb                  # Load a ROM
cargo run -- path/to/rom.gb --debugger       # Load with debugger
cargo run -- path/to/rom.gb --headless       # Headless debugger (HTTP API)
cargo run -- path/to/rom.gb --boot-rom path/to/dmg_boot.bin  # Run with boot ROM
cargo check                                  # Type check
cargo test -p missingno-gb                 # Run core tests (fast, no GUI deps)
cargo test                                   # Run all workspace tests
cargo clippy                                 # Lint
cargo fmt                                    # Format
```

## Testing

- Always run tests against missingno-gb: `cargo test -p missingno-gb`. Do not run `cargo test` against the whole workspace unless specifically asked.
- For regression checking, use `./scripts/test-report.sh --diff` instead of raw `cargo test`. It generates structured reports with baseline comparison and saves them to `receipts/test-reports/`.
- To save a baseline before experimenting: `./scripts/test-report.sh --save-baseline`. Always save a baseline from `main` (or the known-good state) before making changes, so `--diff` has an accurate reference point.
- To run a specific test with the boot ROM: `DMG_BOOT_ROM=<path> cargo test -p missingno-gb <test_name>`. Boot ROMs are proprietary — ask the user for the path, never commit them. Only use on targeted tests; the boot ROM adds significant startup time per test, making full-suite runs impractical.
- After any fix, verify no regressions before committing.

## Emulation Philosophy

- **Hardware fidelity**: Model the hardware as closely as possible so correct behavior emerges naturally. Avoid hacks and special-case workarounds — if something needs a hack to work, the underlying model is wrong.
- **Code as documentation**: The code should teach the reader how the hardware works. Use Rust's type system — enums, newtypes, descriptive variant names — to make structure and intent obvious from the code itself, not from comments. Strike a balance between clarity and jargon; assume the reader is a competent programmer but not necessarily a domain expert in the specific hardware.
- **Hardware over emulator comparisons**: Other emulators are reference material, not ground truth. Always attribute emulator-sourced findings explicitly ("SameBoy does X") rather than as hardware fact. For trace comparisons, prefer the GateBoy adapter over other emulators — it is die-photo-derived and closest to hardware.
- **Data-driven debugging**: Use available data resources (PPU timing model spec, gb-ctr, dmg-sim, gb-propagation-delay-analysis, gbtrace, gb-timing-data, slowpeek) rather than reasoning about behavior from code alone. Observe first, hypothesize second.
- **Flag spec gaps, don't paper over them**: When the PPU timing model spec (`receipts/ppu-overhaul/reference/ppu-timing-model-spec.md`) doesn't cover a behaviour but a dmg-sim run could provide the measurement, raise it with the user so the spec can be extended. Do not fall back to emulator source or hand-wave the answer.
- **Future cores**: These principles apply to any emulation core added to the project, not just Game Boy.

## Investigation hygiene

- **Spec gaps are work for the user, not a reason to pivot.** When a `/research` receipt returns `Confidence: spec-gap` with a concrete dmg-sim measurement target (named signals, named ROM, sub-dot offsets), surface the measurement request to the user and ask them to run it. Do not silently pivot to a different problem — the user actively wants the spec extended, and routing around gaps means working with incomplete data on every downstream decision.
- **Don't claim a spec defect without checking existing resolutions.** Before framing a measurement-vs-spec mismatch as "spec is wrong" or "spec contradicts evidence", FIRST check `receipts/ppu-overhaul/spec-gaps/` for an existing doc covering this scenario, AND read the named spec section directly. Many apparent contradictions are already resolved out-of-band, often by FST measurement that corroborates the spec. The cost of the two checks is ~2 minutes; the cost of asking the user to do an out-of-band review of an already-resolved issue is much higher. This applies to `/research` subagents (don't return `Confidence: spec-gap` without checking spec-gaps/ for prior resolution) and to the dispatcher (don't relay subagent claims of spec-defect without source-verification).
- **Subagent receipts are starting points, not load-bearing claims.** Receipts compress information — they can elide qualifiers, miss adjacent paragraphs that pre-empt the apparent finding, or summarise a measurement without flagging that it was at the wrong sub-phase. When a downstream decision (open a spec-gap, refute a hypothesis, claim a contradiction, declare an investigation resolved) rides on a specific cited line — spec section, code path, FST timestamp, trace entry index — open the source and read it directly before acting.
- **Prior investigation receipts are background, not authority.** The `receipts/investigations/` archive is a record of past attempts and decisions. Constants, file:line citations, and structural claims in old receipts go stale fast — missingno's PPU/CPU subsystems are refactored frequently. Use prior receipts to understand *what was tried and why*, not to import specific facts. Verify any concrete code claim against the current source before relying on it. When briefing a `/research` subagent, frame prior receipts as "background, may be stale" and ask for current-code verification rather than citation.
- **Verify items before user-attention lists.** When writing "for you to verify" or "open questions" lists for the user, verify each item locally first. The user's attention is the constrained resource — items in those lists should be ones genuinely uncheckable from the dispatcher's side (e.g., requires hardware, requires running dmg-sim, requires deep domain expertise). If a claim ends up self-resolving when checked, drop it from the list rather than keeping it as "open".

## Code Style (committed code in `crates/`)

These rules apply to all committed Rust code — production code, tests, and doc-comments. They override generic "explain everything" instincts.

- **Comments are sparse.** Default to no comment. Add one only when WHY is non-obvious — a hidden constraint, a subtle invariant, a workaround. Don't explain WHAT the code does; well-named identifiers do that. One short line max; never multi-paragraph docstrings or multi-line block comments.
- **Reference hardware via gate names, not spec § numbers.** When a comment ties code to hardware, name the gate (`NYXU`, `RYDY`, `CATU`) and what it does in one phrase. **Never write `§6.12` or `spec §X.Y` in committed code** — the spec gets renumbered, the concordance moves, and stale section refs rot. Spec section numbers belong in the spec, in the gate concordance, and in receipts — not in `crates/`.
- **No narration of verification outcomes.** Don't write comments like "this matches GateBoy", "fixes test X", "per the April 24 investigation". Those belong in commit messages and PR descriptions, which decay with code less destructively than rotting comments do.
- **No `// added for …` / `// removed because …` provenance comments.** Git blame answers those questions.
- **When invoking `/design` or `/implement`**, these skills run in-context on the main agent — meaning your conversation memory IS the brief, and the rules above need to actively bind your output. Do not paste spec-section references into the brief — if you write `§6.12` while in design/implement mode, you will paste it into doc-comments. Re-read this Code Style section when entering implement mode if you've drifted.

## Architecture

The project is a Cargo workspace with crates under `crates/`:

- **`crates/missingno-gb/`** (`missingno-gb`) — Core emulation library. No GUI dependencies (only `bitflags` and `rgb`). Contains:
  - **`crates/missingno-gb/src/`** — Core emulation. `GameBoy` owns all hardware components directly (`Cpu`, `Ppu`, `Audio`, `Joypad`, `Timers`, `Dma`, `ExternalBus`, `HighRam`, etc.). `GameBoy::step()` executes one instruction and returns a `StepResult` with `new_screen` and `dots` (T-cycle count).
  - **`crates/missingno-gb/src/debugger/`** — Debugging backend. Wraps `GameBoy` with breakpoints, stepping, disassembly, and a T-cycle counter.
  - **`crates/missingno-gb/tests/accuracy/`** — Integration tests (ROM-based accuracy tests).
- **`crates/missingno/`** (`missingno`) — Iced 0.14 GUI binary. Elm architecture (`Message` → `update()` → `view()`), wgpu shader rendering, cpal audio output via lock-free ring buffer. Lives in `crates/missingno/src/app/`.

### Instruction Execution

`GameBoy::step()` in `crates/missingno-gb/src/execute.rs` runs one instruction in two phases:

1. **Fetch/decode**: Reads the opcode byte, ticks hardware, then reads operand bytes one at a time (ticking hardware after each). `operand_count()` determines byte count from the opcode alone. The buffered bytes are passed to `Instruction::decode()`.
2. **Process**: The `Cpu` state machine (`crates/missingno-gb/src/cpu/mcycle/`) yields one `DotAction` per dot for post-decode work (memory reads/writes, internal cycles). The step loop executes each action and ticks hardware via `next_dot()`.

The M-cycle logic is split across three files in `crates/missingno-gb/src/cpu/mcycle/`:
- `mod.rs` — `DotAction` enum, `BusDot` ring counter model, `BusAction` enum, and `next_dot()` method
- `build.rs` — Constructs the action sequence for each instruction type
- `apply.rs` — Pure CPU mutations (ALU, flags, DAA, etc.)

### Clock Model and Phase Architecture

The Game Boy's master clock produces alternating edges. On hardware, each edge triggers specific circuits — there is no inherent "first" or "second" edge within a dot. The CPU and PPU are clocked by the same master clock and tick in lockstep.

**Emulator model**: `execute.rs` alternates `rise()` and `fall()` phase methods. One dot = one `rise()` + one `fall()`. These are master-clock edges (`ck1_ck2` rising and falling — see PPU timing spec §1.1). The CPU and PPU both do work on each edge:
- `rise()`: PPU pixel output (`ppu.on_master_clock_rise()`), CPU state advance (`next_dot()`), CPU reads
- `fall()`: PPU fetcher/control (`ppu.on_master_clock_fall()`), CPU bus writes

**There is no ordering between rise and fall.** They are alternating edges in a continuous clock. Do not reason about "rise happens before fall" — think about which edge a DFF captures on and which edge reads it.

**DFF visibility**: When a DFF captures on edge E, the output holds that value until the next capture. No "same edge" vs "next edge" distinction. `DffLatch`: `write()` sets pending, `tick()` resolves to output (capture edge), `output()` reads last captured value.

**CPU bus writes**: Action determined in `rise()` via `next_dot()`, executed in `fall()` via `drive_ppu_bus()`. DFF9 registers (LCDC, SCY, SCX) use the "early write path" before `ppu.on_master_clock_fall()`. DFF8 palette registers (BGP, OBP0, OBP1) use early write + `tick_palette_latches()` inside `ppu.on_master_clock_fall()`. To add registers to the early write path, update the match at `execute.rs` line ~398.

**GateBoy conventions**: 8 sub-phases (A-H) per M-cycle, 2 per dot. `mcycle_phase` packs ring counter DFFs: 0x0C=B, 0x0F=D, 0x03=F, 0x00=H. `_evn` DFFs latch on EVEN edges; `_odd` on ODD. CPU register writes latch at DELTA_GH (first visible at phase H). See `receipts/research/` for phase mapping docs.

**Common pitfalls**: (1) Never frame timing hypotheses as "move X before/after Y in rise/fall" — think about DFF capture edges and combinational read points. (2) Multi-stage pipeline fixes: if a fix has zero effect, check whether another pipeline stage compensates — both stages may need fixing together.

### Key Patterns

- **Flat component ownership**: `GameBoy` owns all hardware components as separate fields (`cpu`, `ppu`, `audio`, `timers`, `interrupts`, `dma`, etc.) so subsystems can be borrowed independently.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `crates/missingno-gb/src/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `Ppu` holds an `Option<Rendering>` — `None` when the LCD is off (hardware reset state), `Some(Rendering)` when on. `Rendering` persists through both active display and VBlank (matching hardware where pixel circuits are always present when LCD is on). Modes are derived from `video.vblank` and scanning state within `Rendering`. Draws pixels one at a time with cycle-accurate timing.
- **Propagation delay analysis**: The sibling project [`gb-propagation-delay-analysis`](https://github.com/ajoneil/gb-propagation-delay-analysis) (local clone: `receipts/resources/gb-propagation-delay-analysis/`) provides static analysis of GateBoy's netlist — signal races, deep combinatorial paths, and propagation delays. Key outputs in `receipts/resources/gb-propagation-delay-analysis/output/`: `race_pairs_report.md` (observable effects by symptom), `critical_paths_report.md` (deepest paths), `signal_concordance.md` (GateBoy cell names ↔ Pan Docs names). For one-dot timing discrepancies, check race pairs first.

- **Execution tracing (gbtrace)**: The sibling project [`gbtrace`](https://github.com/ajoneil/gbtrace) (local clone: `receipts/resources/gbtrace/`) defines a standardised format for recording and comparing Game Boy emulator execution state across multiple emulators. Tracked emulators: gambatte, gateboy, docboy, missingno, sameboy. DocBoy traces at T-cycle granularity. Missingno integrates this behind the `gbtrace` feature flag on `missingno-gb`:
  - **Capturing traces** — `tests/accuracy/common/` wraps `GameBoy` in `TestRun`, which optionally traces each `step()`:
    ```bash
    GBTRACE_PROFILE=gbmicrotest cargo test -p missingno-gb --features gbtrace -- <test_name>
    ```
    Writes to `receipts/traces/<rom_name>.gbtrace`. Profiles are per-suite TOML files in `receipts/resources/gbtrace/test-suites/*/profile.toml`.
  - **gbtrace CLI** — Build with `cargo build -p gbtrace --features cli` from `receipts/resources/gbtrace/`. Key commands:
    - `gbtrace info <file>` — trace metadata summary.
    - `gbtrace query <file> --where pc=0x0150` — find entries matching conditions (`--context N`, `--max N`, `--last N`, `--range START..END`, `--fields`). Multiple `--where` args for AND conditions (not comma-separated).
    - `gbtrace diff <a> <b>` — compare traces (`--sync`, `--fields`, `--exclude`, `--summary`).
    - `gbtrace frames <file>` — frame boundaries from LY.
    - `gbtrace render <file> -o <dir>` — render LCD frames to PNG (`--frames 1,3,5`).
    - `gbtrace convert <file>` — convert JSONL to native `.gbtrace` format.
  - **Reference traces**: Hosted at [ajoneil.github.io/gbtrace](https://ajoneil.github.io/gbtrace/) with manifests for 15 test suites. URL pattern: `tests/{suite}/{test}_{emulator}_{status}.gbtrace`. Use the `/compare-traces` skill for structured comparison and individual trace inspection.
- **Boot ROM support**: Optional DMG boot ROM via `--boot-rom <path>` (CLI) or `DMG_BOOT_ROM=<path>` (tests). Boot ROMs are proprietary — never commit them. Without one, post-boot initialization is used. Only use on targeted tests (adds significant startup time).
- **Config**: `settings.ron` and `recent.ron` in platform config dir via `dirs` crate. Uses `jiff` (not `chrono`) for timestamps.

### Data Sources for Debugging and Research

When investigating emulator issues, these data sources are available in priority order. Always check existing data before generating new diagnostics.

1. **PPU timing model spec** (`receipts/ppu-overhaul/reference/ppu-timing-model-spec.md`): Gate-level hardware behaviour for the DMG PPU, collated from dmg-sim measurements, netlist analysis, and propagation-delay analysis. Canonical hardware reference for PPU timing. Companion files: `ppu-signal-concordance.md` (gate name ↔ semantic role), `spec-conventions.md` (authoring conventions). **If a PPU question isn't covered by the spec but a dmg-sim measurement could answer it, flag this to the user so the spec can be updated** — do not fall back to emulator source.
2. **gekkio's gb-ctr** (Game Boy Complete Technical Reference, https://gekkio.fi/files/gb-docs/gbctr.pdf): Detailed, reliable hardware reference for the whole console. Primary written reference outside the PPU spec.
3. **dmg-sim** (`receipts/resources/dmg-sim/`): Gate-level SystemVerilog simulation built from the DMG-CPU B netlist. Run with `scripts/dmg-sim-observe.sh <rom> [seconds] [output_dir]` to capture an FST waveform (`receipts/traces/dmg-sim/<rom>.fst`, viewable in GTKWave). Use when the PPU timing model spec lacks detail the spec should contain — and update the spec afterwards.
4. **Propagation delay analysis** (`receipts/resources/gb-propagation-delay-analysis/`): Signal races and deep combinatorial paths. See Key Patterns above.
5. **Hardware documentation**: Pan Docs, TCAGBD, hardware manuals. Useful for non-PPU behaviour and as cross-reference.
6. **Cross-emulator execution traces** (gbtrace): 5 emulators, 15 test suites. Use for both `diff` and individual inspection. **Prefer the GateBoy adapter** — die-photo-derived, closest to hardware. Fall back to Gambatte / SameBoy / DocBoy only when GateBoy does not pass the test or has no trace for it. See Key Patterns above.
7. **Hardware timing measurements** (`receipts/resources/gb-timing-data/`): Empirical cycle-level data from real hardware via Slowpeek. Campaigns cover PPU timing (mode 3 duration, sprite penalties, OAM/VRAM lock boundaries) and timer subsystem timing (DIV phase, TIMA increment). Results are CSV files with multi-dimensional sweep data. **Status: data collection in progress** — check `receipts/resources/gb-timing-data/campaigns/` for available campaigns.
8. **Test ROM sources**: Assembly source reveals exactly what tests measure and what expected values mean.
9. **Hardware test harness** (`receipts/resources/slowpeek/`): Programmable harness for cycle-precise measurements on real Game Boy hardware via interrupt-driven sweeps. **Status: emulator-only for now; hardware serial bridge in development.** Note when a Slowpeek test would provide the definitive answer, but do not attempt hardware mode yet.

### Debugger

- **Pane system**: `crates/missingno/src/app/debugger/panes.rs` manages a `pane_grid` of `DebuggerPane` variants (Screen, Instructions, Tiles, TileMap, Sprites, Audio). Each pane is a separate module with a struct (e.g. `ScreenPane`, `InstructionsPane`), a `content()` method returning `pane_grid::Content`, and optionally a `Message` enum. Register new panes by adding to `DebuggerPane` enum, `PaneInstance` enum, `construct_pane()`, `view()`, `available_panes()`, and `Display` impl.
- **Input recording**: `crates/missingno-gb/src/recording.rs` defines the `Recording` data model (ROM header + initial state + input events).

### Resources

External resources live in `receipts/resources/`. Clone or download whatever you need there — it's gitignored. These sibling projects are referenced throughout the skills and documentation:

| Directory | Repository | Description |
|-----------|------------|-------------|
| `dmg-sim` | https://github.com/msinger/dmg-sim | Gate-level SystemVerilog simulation of DMG-CPU B (Icarus Verilog) — primary source for PPU timing measurements |
| `gb-propagation-delay-analysis` | https://github.com/ajoneil/gb-propagation-delay-analysis | GateBoy netlist analysis — signal races, critical paths, propagation delays |
| `gbtrace` | https://github.com/ajoneil/gbtrace | Execution trace capture, diff, and render across emulators |
| `gb-timing-data` | https://github.com/ajoneil/gb-timing-data | Cycle-level hardware timing measurements |
| `slowpeek` | https://github.com/ajoneil/slowpeek | Cycle-precise hardware test harness |

Primary hardware documentation (not cloned — fetch as needed):

| Resource | URL | Description |
|----------|-----|-------------|
| PPU timing model spec | `receipts/ppu-overhaul/reference/ppu-timing-model-spec.md` | Canonical DMG PPU timing reference, collated from dmg-sim, netlist, and propagation-delay analysis |
| gb-ctr | https://gekkio.fi/files/gb-docs/gbctr.pdf | Gekkio's Game Boy Complete Technical Reference |
