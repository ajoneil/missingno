# missingno-gb — Game Boy (DMG) methodology and core internals

Core-specific methodology, ground-truth hierarchy, and internals for the Game
Boy family. `missingno-gb` is the **shared-silicon base**: the generic
`Console<M>` core plus the DMG model. `missingno-gbc` is a superset built on top
of it (see `crates/missingno-gbc/AGENTS.md` for the CGB delta). The shared
skill-system rules, agent infrastructure, and workflow discipline live in the
repository-root `AGENTS.md` — this file is the Game Boy *core* detail that a
non-GB session does not need to load.

## Ground-truth hierarchy

Primary references, in order — check these before generating new diagnostic data:

1. **The DMG Timing Specification** (https://ajoneil.github.io/dmg-timing-spec/): the published gate- and signal-level spec of the whole DMG-CPU B — PPU pipeline and modes, APU per-channel counters, CPU-visible register boundaries, DMA, timers, interrupt dispatch, clock tree — collated from dmg-sim measurements and netlist analysis, with a gate concordance chapter. Canonical hardware reference for any gate-level question. Source: https://github.com/ajoneil/dmg-timing-spec (clone into `receipts/resources/` to grep its `src/` when searching). **If a question isn't covered by the spec but a dmg-sim measurement could answer it, flag this to the user so the spec can be extended** — do not fall back to emulator source.
2. **gekkio's gb-ctr** (Game Boy Complete Technical Reference, https://gekkio.fi/files/gb-docs/gbctr.pdf): Detailed, reliable hardware reference for the whole console. Primary written reference outside the PPU spec.
3. **dmg-sim** (`receipts/resources/dmg-sim/`): Gate-level SystemVerilog simulation built from the DMG-CPU B netlist. Run with `scripts/dmg-sim-observe.sh <rom> [seconds] [output_dir]` to capture an FST waveform (`receipts/traces/dmg-sim/<rom>.fst`, viewable in GTKWave). Use when the DMG Timing Specification lacks detail it should contain — and update the spec afterwards. Runs are slow and waveform analysis is specialised — **prefer surfacing the specific signal to observe to the user** rather than invoking dmg-sim directly.
4. **Propagation delay analysis** (`receipts/resources/gb-propagation-delay-analysis/`): Signal races and deep combinatorial paths. See Key Patterns below.
5. **Hardware documentation**: Pan Docs, TCAGBD, hardware manuals. Useful for non-PPU behaviour and as cross-reference.
6. **Cross-emulator execution traces** (morepork): 4 behavioural emulators (SameBoy, DocBoy, gambatte, missingno), 17 test suites. Use for both `diff` and individual inspection. References corroborate but never ground a hardware finding — prefer SameBoy, then DocBoy, then gambatte. Reference traces are generated locally (`make traces-<suite> EMUS=...` in the morepork clone). See Key Patterns below.
7. **Hardware timing measurements** (`receipts/resources/gb-timing-data/`): Empirical cycle-level data from real hardware via Slowpeek. Campaigns cover PPU timing (mode 3 duration, sprite penalties, OAM/VRAM lock boundaries) and timer subsystem timing (DIV phase, TIMA increment). Results are CSV files with multi-dimensional sweep data. Check the local clone's `campaigns/` directory for available data before planning around this source.
8. **Test ROM sources**: Assembly source reveals exactly what tests measure and what expected values mean.
9. **Hardware test harness** (`receipts/resources/slowpeek/`): Programmable harness for cycle-precise measurements on real Game Boy hardware via interrupt-driven sweeps. Note when a Slowpeek test would provide the definitive answer; check whether the hardware serial bridge is available before attempting hardware mode.

## Investigation hygiene (DMG spec workflow)

- **Spec gaps are work for the user, not a reason to pivot.** When a `/research` receipt returns `Confidence: spec-gap` with a concrete dmg-sim measurement target (named signals, named ROM, sub-dot offsets), surface the measurement request to the user and ask them to run it. Do not silently pivot to a different problem — the user actively wants the spec extended, and routing around gaps means working with incomplete data on every downstream decision.
- **Don't claim a spec defect without checking existing resolutions.** Before framing a measurement-vs-spec mismatch as "spec is wrong" or "spec contradicts evidence", FIRST check `receipts/ppu-overhaul/spec-gaps/` for an existing doc covering this scenario, AND read the named spec section directly. Many apparent contradictions are already resolved out-of-band, often by FST measurement that corroborates the spec. The cost of the two checks is ~2 minutes; the cost of asking the user to do an out-of-band review of an already-resolved issue is much higher. This applies to `/research` subagents (don't return `Confidence: spec-gap` without checking spec-gaps/ for prior resolution) and to the dispatcher (don't relay subagent claims of spec-defect without source-verification).
- **Grep failing test names across spec-gaps and prior investigations before forming hypotheses.** A new investigation's first scoping step should `grep -r '<failing_test_name>' receipts/ppu-overhaul/spec-gaps/ receipts/investigations/*/summary.md`. Read direct matches in full — both OPEN and CLOSED spec-gap docs, and prior-investigation summaries. A test named as a related target in a CLOSED spec-gap doc, or anticipated by a prior investigation that didn't get to it, is a load-bearing fact that compresses out of "what prior work covered" summaries. Skipping this step costs multiple subagent dispatches re-deriving framings that the existing docs already contain.

## Instruction Execution

`Console::step()` (`crates/missingno-gb/src/execute.rs`) drives `execute_phase()` — one master-clock edge per call — until the CPU returns to an instruction boundary. The CPU's M-cycle machinery lives in `crates/missingno-gb/src/cpu/mcycle/`, split by role: `fetch.rs` (opcode/operand fetch and `Instruction::decode`, including `operand_count()`), `scheduler.rs` (M-cycle/T-cycle sequencing, bus arbitration parking), `execute.rs` (per-phase execution steps), `isr.rs` (interrupt dispatch), `build.rs` (action sequences per instruction type), `apply.rs` (pure CPU mutations — ALU, flags, DAA), `types.rs` (`BusAction` and friends). Read the module before trusting any finer-grained claim here — this split has been reshaped more than once.

## Clock Model and Phase Architecture

The Game Boy's master clock produces alternating edges. On hardware, each edge triggers specific circuits — there is no inherent "first" or "second" edge within a dot. The CPU and PPU share the master clock; at single speed they tick in lockstep, and at CGB double speed the `MasterClock`'s ÷2 divider advances the dot edge on alternate CPU edges (the divider is owned by `Chassis.clock`; KEY1 mutates it at the speed switch).

**Emulator model**: `execute_tcycle()` in `execute.rs` advances one CPU T-cycle (its rise then its fall) as straight-line flow. `MasterClock::tcycle_schedule()` names the dot edges the T-cycle carries — `FullDot` at ÷1; at ÷2 exactly one dot edge on the rise, alternating `DotRiseOnRise`/`DotFallOnRise` (no other combination is representable). One dot = one rise + one fall (`ck1_ck2` edges — see the DMG Timing Specification's Clock Tree chapter). Public single-stepping is per T-cycle (`step_tcycle()`); there is no half-edge stepping. The speed-switch blackout drives held DOT edges through the clock's `Held` arm in its own loop, outside the fused path.
- rise: PPU pixel output, CPU state advance, CPU reads
- fall: PPU fetcher/control, memory write commit

**There is no ordering between rise and fall.** They are alternating edges in a continuous clock. Do not reason about "rise happens before fall" — think about which edge a DFF captures on and which edge reads it.

**DFF visibility**: When a DFF captures on edge E, the output holds that value until the next capture. No "same edge" vs "next edge" distinction. `DffLatch`: `write()` sets pending, `tick()` resolves to output (capture edge), `output()` reads last captured value.

**CPU bus writes**: PPU register writes drive the bus on the rise edge at T-cycle 2 (`drive_ppu_bus`, CUPA-high — registers latch combinationally while CUPA is high); memory commits at CUPA-falling in the fall path (`commit_write`). All PPU registers route through `ppu.write_register`; per-fall register-file latching is the register file's own `tick()` (staged `DffLatch` writes resolve there). Read `drive_ppu_bus` in `memory.rs` and the fall path in `execute.rs` before extending — the exact edge assignments are corpus-pinned.

**Common pitfalls**: (1) Never frame timing hypotheses as "move X before/after Y in rise/fall" — think about DFF capture edges and combinational read points. (2) Multi-stage pipeline fixes: if a fix has zero effect, check whether another pipeline stage compensates — both stages may need fixing together. (3) Never say "integer-dot model" / "integer-dot rounding" / "discrete-dot pipeline can't represent X" / "busdot=N" as if those were limits of the emulator. Rise + fall per dot IS half-dot resolution — every hardware edge has a corresponding emulator edge; there is no precision being lost. Frame divergences as "which edge captures what", never as "the dot count is off by N." This is the root "approximation artifact" rule instantiated for DMG — here the model's resolution provably matches the hardware's. (4) Spec sub-dot phases (e.g. "WODU↑ at dot 1.5150", "XYMU.q↑ at +0.481 dots") are *edge identifiers*, not fractional dot counts to round. Translate them to edge labels (rise of dot N, fall of dot N) before reasoning. A divergence stated as "rounded to the wrong integer dot" is almost always actually "fired on the wrong edge of the same dot, or on an adjacent edge of the next/prior dot."

**Wrong → right framing examples**:
- ✗ "Our integer-dot model collapses the sub-dot regimes" → ✓ "Our `capture_voga` evaluates WODU and captures VOGA on the same rise edge; hardware splits these across `fall(N)` (WODU rises) and `rise(N+1)` (VOGA captures)."
- ✗ "Mode-3 ends 1 dot late due to integer-dot rounding" → ✓ "XYMU.q clears at `rise(N+1)` but hardware clears it at `rise(N)`; the missing edge is VOGA's same-dot ALET-rising capture."
- ✗ "The dot-2 snapshot can't represent the bus-driver settling window" → ✓ "Our snapshot fires at `fall(busdot=2)`; the bus-driven mode bits transition between `rise(busdot=2)` (XYMU clear) and `fall(busdot=3)` (BUKE), so the right edge to sample on is the BUKE fall, not the dot-2 fall."

## Key Patterns

- **Chassis + model composition**: `Console<M> = { chassis: Chassis<M>, model: M }`. `Chassis` holds the shared hardware as separate fields (`cpu`, `ppu`, `audio`, `timers`, `interrupts`, `dma`, …) so subsystems borrow independently, and — because it names only `M`'s associated types, never `M` itself — model hooks can take `&mut Chassis<Self>` while the model is borrowed (disjoint borrows). Console-specific behaviour (CGB HDMA, speed switch) lives in the model, called at named seam points in the step loop.
- **Memory-mapped I/O**: `MappedAddress::map()` translates raw addresses to typed enum variants, routing reads/writes to the correct subsystem.
- **Enum-based MBC dispatch**: `Mbc` enum in `crates/missingno-gb/src/cartridge/mbc/mod.rs` with variants for all known Game Boy cartridge types (NoMbc, MBC1-3, MBC5-7, HuC1, HuC3), selected at runtime from cartridge header byte 0x147. ROM data is owned by `Cartridge` and passed to MBC `read()` methods as `&[u8]`.
- **PPU state machine**: `Ppu<P: PpuModel>` holds `pixel_pipeline: Option<Rendering<P>>` — `None` when the LCD is off (hardware reset state), `Some` when on. `Rendering` persists through both active display and VBlank (matching hardware where pixel circuits are always present when LCD is on); modes derive from the video-control dividers/line state plus scanning state. Draws pixels one at a time with cycle-accurate timing.
- **Propagation delay analysis**: The sibling project [`gb-propagation-delay-analysis`](https://github.com/ajoneil/gb-propagation-delay-analysis) (local clone: `receipts/resources/gb-propagation-delay-analysis/`) provides static analysis of the DMG-CPU die netlist — signal races, deep combinatorial paths, and propagation delays. Key outputs in `receipts/resources/gb-propagation-delay-analysis/output/`: `race_pairs_report.md` (observable effects by symptom), `critical_paths_report.md` (deepest paths), `signal_concordance.md` (netlist cell names ↔ Pan Docs names). For one-dot timing discrepancies, check race pairs first.
- **Execution tracing (morepork)**: The sibling project [`morepork`](https://github.com/ajoneil/morepork) (local clone: `receipts/resources/morepork/`) defines a standardised format for recording and comparing emulator execution state across multiple emulators. Tracked emulators: gambatte, docboy, missingno, sameboy. DocBoy traces at T-cycle granularity. Missingno integrates this behind the `morepork` feature flag on `missingno-gb`:
  - **Capturing traces** — the trace header is authored from the core's hardware state schema (`missingno-core`'s `SystemStateSchema`, per `crates/missingno-gb/src/trace.rs`): the columns are the schema's Tier-1 observable fields plus the trace-only observations (`op_addr`, `pix`, write taps), typed straight from the schema — there is no per-suite field catalogue any more. `TestRun<M>` (in `crates/missingno-gb/src/test_support.rs`, behind the `test-support` feature; `tests/accuracy/common/` re-exports it) wraps a console and traces each `step()` when `MOREPORK_PROFILE` is set (any value enables). `MOREPORK_TRIGGER` (`tcycle`/`instruction`, default instruction) sets the cadence; `MOREPORK_SCOPE` (`observable`/`full`, default observable) selects the tier depth — `full` adds the schema's Tier-2a deep state. CGB capture: `missingno-gbc` has its own `morepork` feature, `load_cgb_rom_traced`, and the `MOREPORK_CAPTURE_ROM` harness. DMG example:
    ```bash
    MOREPORK_PROFILE=1 MOREPORK_TRIGGER=tcycle MOREPORK_SCOPE=full \
      cargo test -p missingno-gb --features morepork -- <test_name>
    ```
    Writes to `receipts/traces/<rom_name>.morepork`. A quick two-trace divergence smoke: `cargo run -p missingno-gb --example trace-capture --features morepork -- <out.morepork> [initial_a_hex]`.
  - **morepork CLI** — Build with `cargo build -p morepork --features cli` from `receipts/resources/morepork/`. Key commands:
    - `morepork info <file>` — trace metadata summary.
    - `morepork query <file> --where pc=0x0150` — find entries matching conditions (`--context N`, `--max N`, `--last N`, `--range START..END`, `--fields`). Multiple `--where` args for AND conditions (not comma-separated).
    - `morepork diff <a> <b>` — compare traces (`--sync`, `--fields`, `--exclude`, `--summary`).
    - `morepork frames <file>` — frame boundaries from LY.
    - `morepork render <file> -o <dir>` — render LCD frames to PNG (`--frames 1,3,5`).
    - `morepork convert <file>` — convert JSONL to native `.morepork` format.
  - **Reference traces**: generate locally — the published CDN corpus predates format v3 and is orphaned. In `receipts/resources/morepork/`: `make adapters`, then `make traces-<suite> EMUS=sameboy` (output in `build/traces/<suite>/`, named `<test>_<emu>_<model>_<status>.morepork`). Generate only what the investigation needs. Use the `/compare-traces` skill for structured comparison and individual trace inspection.
- **Boot ROM support**: Optional boot ROM via `--boot-rom <path>` (CLI) or `DMG_BOOT_ROM=<path>` / `CGB_BOOT_ROM=<path>` (tests). Boot ROMs are proprietary — never commit them. Without one, post-boot initialization is used. Only use on targeted tests (adds significant startup time).

## Resources

| Directory / Resource | Location / URL | Description |
|----------------------|----------------|-------------|
| `dmg-sim` | https://github.com/msinger/dmg-sim | Gate-level SystemVerilog simulation of DMG-CPU B (Icarus Verilog) — primary source for PPU timing measurements |
| `gb-propagation-delay-analysis` | https://github.com/ajoneil/gb-propagation-delay-analysis | DMG-CPU die netlist analysis — signal races, critical paths, propagation delays |
| `gb-timing-data` | https://github.com/ajoneil/gb-timing-data | Cycle-level hardware timing measurements |
| `slowpeek` | https://github.com/ajoneil/slowpeek | Cycle-precise hardware test harness |
| DMG Timing Specification | https://ajoneil.github.io/dmg-timing-spec/ | Published gate-level spec of the DMG-CPU B (PPU, APU, CPU boundaries, DMA, interrupts, timers). Source: https://github.com/ajoneil/dmg-timing-spec |
| gb-ctr | https://gekkio.fi/files/gb-docs/gbctr.pdf | Gekkio's Game Boy Complete Technical Reference |
