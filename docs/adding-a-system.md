# Adding an emulated system

Missingno is built so a new console family plugs into a fixed set of seams and
earns whole subsystems by filling them. This document is the checklist spine:
what implementing a new system costs, and what each seam surface buys. Every
claim here is checkable against the trait it names — trust the seams, but verify
signatures against the source before building on them.

The seam vocabulary and its behavioural traits live in **`missingno-core`**
(`crates/missingno-core/src/`); the headless debugger that consumes them lives
in **`missingno-debugger`**; the GUI's family-registration layer lives in
**`crates/missingno/src/app/system/`**. Those three locations recur throughout.

## Two different axes

Don't confuse the two kinds of "new system":

- **A variant within a family, sharing silicon** — the DMG↔CGB axis. Both are
  `Console<M>` with the divergences carried by the `Model` / `PpuModel` /
  `ApuSpec` traits in `missingno-gb`. If the new machine shares the SM83 core
  and PPU/APU lineage (Super Game Boy is already modelled this way; a
  hypothetical Mega Duck would fit too), implement a new `Model` and stay
  inside the family. Read the extensive doc comments on those traits — every
  associated const and hook documents the exact hardware divergence it exists
  to carry.
- **A new family entirely** (Master System, NES, a future Game Gear) — a new
  core crate plus a frontend family registration. The rest of this document is
  about this axis.

## The invariant contract every core owes

Use the available evidence to reach the highest accuracy possible — **and the
mechanism that achieves it is a per-core decision**. The Game Boy runs a
fused-T-cycle lockstep because gate-level ground truth (dmg-sim, the netlist)
makes sub-cycle ordering verifiable and the suites demand it. A system whose
best evidence is test-ROM-granular must not pay for — or claim — fidelity
nobody can check; its methodology doc (`crates/<crate>/AGENTS.md`) picks the
internal quantum (dot, colour clock, master-cycle slice, instruction-granular
catch-up), names its ground-truth hierarchy, and defends the quantum against
that tier. The VCS doc is the worked example: Sim2600 for the CPU/TIA,
datasheet/schematics for the RIOT.

What is NOT per-core is the contract the seam depends on. Any internal mechanism
must provide:

1. **Determinism** — same ROM + inputs → bit-exact execution (replay,
   tracing, and bisection depend on it).
2. **Instruction-boundary stepping** for the debugger's step/breakpoints, and
   a **restore boundary** for save states (capture and restore at an
   instruction boundary; error, never panic, off-boundary).
3. **On-demand bus observability** without behaviour change (watchpoints,
   code/data logging, trace capture).
4. **Side-effect-free inspection reads** (disassembly, memory panes).
5. **A cheap owned per-frame snapshot** for the running debugger view.
6. **Budgeted frame stepping** with a stall guard, frames as data.
7. **Committable test oracles in CI from day one** — accuracy claims live
   in tests.

## The two seam traits, and what they buy

The whole seam is two object-safe traits in `missingno-core`'s `system.rs`. A
family builds a `Box<dyn SystemConsole>`; the shell drives it, and converts it
into a `Box<dyn SystemDebugger>` (`into_debugger`, fallible — a family with no
debugger backend hands the console back and the shell falls back to plain
emulation).

### `SystemConsole` → the plain emulator and the emu thread

`step_frame` (budgeted), `reset`, `set_control`, `drain_audio_samples` (44.1 kHz
stereo — the seam's fixed rate), `screen_display`, `game_title`, `battery_save`,
and `frame_interval` (the pacing loop's wall-clock). Everything the library, the
emulator screen, and the emulation thread need. Optional surfaces —
`console_switches` (the VCS's difficulty/colour toggles), `uses_monochrome_palette`
(the DMG palette picker), `audio_coupling` (the board's high-pass) — default to
absent, so a family declares only what its hardware has.

### `SystemDebugger` → the whole debugger, both frontends

Implement it and two debuggers light up with no further per-core code:

- **The headless server (`missingno-debugger`).** Its transport-agnostic
  `Session` reads the console entirely through the seam; the HTTP transport
  (scripted/bulk access) and the MCP-over-stdio transport (interactive agent
  use) are both purely generic `Session` calls, with no core-specific code
  outside the one-line factory registry. Registering a core there gives it
  every route and every tool at once.
- **The GUI debugger.** Its pane grid renders exclusively from the typed
  surfaces the seam carries — there is no family-specific pane escape hatch, so
  a family cannot (and need not) add a bespoke pane.

Concretely, each debugger surface is one seam method, and filling it turns that
surface on across both frontends:

| Seam method(s) | Debugger surface it buys |
|---|---|
| `step` / `step_over` / `step_frame`, `set_breakpoint` / `clear_breakpoint` / `breakpoints` | stepping and PC breakpoints |
| `tick_name` + `step_tick` | sub-instruction stepping (a Game Boy dot, a VCS colour clock) — advertised only when `tick_name` is `Some` |
| `peek` + `memory_regions` | the memory hex dump and the named memory map |
| `instruction_set` (a `missingno-core` `InstructionSet`) + `pc` | disassembly; `bank_for` / `present_address` / `locate_bank_window` add bank-prefixed and `bank:window` rows |
| `register_groups` → `sidebar_sections` | the registers view and the whole machine-state sidebar (see below) |
| `watchables` + `add_watch` / `remove_watch` / `watches` | the watch panel, and banked breakpoints where the family exposes pc/bank watchables (a banked stop composes as a pc+bank compound watch — plain `set_breakpoint` is bus-space by contract and rejects a synthetic banked address) |
| `set_wave_capture` + `channel_waves` | the audio scopes / `get_waveforms` — interest-gated, costing nothing until a consumer turns capture on |
| `set_graphics_capture` + `graphics` (a `GraphicsView`: tile atlases, maps, an object table) | the graphics panes / `get_tiles` / `get_objects` — likewise interest-gated |
| `symbols` / `add_symbol` / `cdl_window` + `load_sidecars` / `save_sidecars` | debug-symbol labels and the code/data-log data rows (no-ops for a family with no sidecars) |
| `snapshot` → a `DebugView` (`Box<dyn InspectSnapshot>`) + `running_status` | the per-vblank running view the UI renders while the core runs on the emu thread, without owning the console |
| `family_state` / `as_any_mut` | the family's own typed state, downcast by its panes and any headless extension routes |

Everything above the family's own decode backends is generic. A family with a
disassembler, a register file, and a memory map gets a working debugger; the
graphics and audio surfaces come online as it fills their capture hooks.

### `video_out` → `DisplayTechnology` → authentic rendering

`SystemConsole::video_out` (and the debugger's) returns a `DisplayTechnology` —
a hardware fact the core states, never a presentation coefficient. `Lcd { native,
panel, pixel_aspect }` names the panel class (`PassiveStn` for the DMG's slow
passive-matrix STN, `ActiveTft` for the CGB's faster TFT); `Crt { standard,
pixel_aspect }` names the broadcast standard. The single frontend screen
renderer (`crates/missingno/src/app/screen.rs`) keys its persistence blend and
its cosmetic overlay (an LCD pixel grid vs. CRT scanlines) off that technology,
and aspect-fits by the stated `pixel_aspect`. State the technology and the
console renders authentically; the coefficients stay frontend policy.

### `SystemStateSchema` → save states, traces, and recordings

Author one `SystemStateSchema` (`missingno-core`'s `state.rs`) of hardware-named
fields — Tier-1 `observable` registers (the CPU-visible surface any emulator can
produce), Tier-2a `boundary` deep state named for the silicon (enough to restore
bit-exactly at a boundary) — plus memory spans and a `FrameSpec`. Wire the
`state_schema()` / `read_state()` seam methods and a boundary bridge behind
`save_state()` / `load_state()`. That one schema then drives three surfaces off
the same field vocabulary:

- **Save states** — the `MPSV` state file (`state_file.rs`): every field at one
  instant, carrying its own system id and ROM fingerprint so a restore validates
  the target console and rejects a state for the wrong ROM, wrong system, or
  wrong version.
- **Traces** — the `MPRK` trace container: columns are the schema's fields
  (Tier-1, or Tier-2a with the deep scope) plus a small bridge-owned observation
  set. `crates/missingno-gb/src/trace.rs` is the worked bridge; there is no
  per-suite field catalogue.
- **Recordings** — the `MPRC` recording (`recording.rs`): an initial save state
  plus a frame-indexed input trace with periodic frame-hash checkpoints.
  **Recording and deterministic replay are built entirely on the existing seam**
  (`save_state` / `load_state` / `set_control` / `step_frame`) — a core that
  wires save states gets replay for free, with no new trait methods.

### `sidebar_sections` → the GUI sidebar, `describe_machine`, and `/sections`

A family surfaces its chip state by composing `Section`s from `missingno-core`'s
`inspect.rs` vocabulary (register files, bit tables, sweeps, pair matrices,
pixel strips, colour swatches, rows). `sidebar_sections` defaults to a single CPU
section from `register_groups`; a family overrides it to add its video and system
sections. The same list renders three ways from one authoring: the GUI's
left-column sidebar, the headless `describe_machine` MCP tool, and the HTTP
`/sections` route. This composition is per-system by design — the shared panes
read typed surfaces, but the sidebar's shape is the family's own.

## The shortcut for a plain stepping core

For a core whose debugger is plain instruction stepping (PC breakpoints, one
typed inspection state, indexed frames), don't implement the two seam traits by
hand: implement `SteppingSystem` (`missingno-core`'s `stepping.rs`) — a flat list
of hooks — and the shared `SteppingConsole<S>` / `SteppingDebugger<S>` carry the
seam's control flow. The Master System and NES are the worked consumers; the VCS
adapts its own core-side debugger backend directly instead, and the Game Boy
family implements the seam once (in `missingno-gb`'s `system.rs`, as
`GbConsole<M>`, generic over its `Model`) — consumed unchanged by both the
headless factory and the GUI.

## The frontend family axis: `app/system/`

The core seam is system-agnostic; the GUI still needs a per-family registration
for media handling. Each family registers one `FamilyDescriptor` in the
`FAMILIES` table (`app/system/mod.rs`): a `Platform` variant (the canonical
platform identity — its `name()` / `short_name()` are the only display strings,
and external platform descriptions map into the enum rather than showing raw),
`extensions`, `control_labels` (the family's names for the shared control ids),
an `is_rom` predicate (mutually exclusive across the table), an optional
`title_from_rom` header hook, a `create_console` factory taking a `MediaLoad`,
and an optional `trace` entry point for the `trace` subcommand. The file dialog,
ROM loading, title detection, the library scanner (which stamps the platform),
the bindings UI, and the trace CLI all iterate that table; `family_for` is the
single classification point.

The Game Boy registers **two** platforms — "Game Boy" and "Game Boy Color" —
sharing one factory; the header picks the execution core inside the factory (a
dual-compatible cart boots the CGB core enhanced), so platform identity and
execution core are deliberately decoupled, like a GB cart slotted into a real
GBC.

The GUI's pane registry is likewise family-provided: `pane_family()` is a
required seam method returning a `panes::Family` (its `PaneDescriptor` list names
which generic panes the family registers — Screen plus the Memory/Disassembly
set for a register-dump family; the graphics and audio panes only where the core
fills their capture hooks — and its default layout). Panes may be instanceable
(the Memory pane opens a fresh instance per click; a message can target one
instance so sibling panes are untouched) or single-instance.

## Honest inventory: what is still per-system

Filling the seam buys the subsystems above; these stay a family's own work:

1. **Sidebar composition** — the shared panes read typed surfaces, but a
   family authors its own `sidebar_sections` from the `inspect` vocabulary.
2. **State-schema authoring and its bridge** — the schema, `read_state`, and
   the boundary save/restore are per-system; only the file/trace/recording
   machinery downstream is shared.
3. **The instruction set, if the CPU is new** — implement `InstructionSet`
   (`missingno-core`'s `isa.rs`) for decode-for-display, or reuse a shared one
   (`missingno-6502` serves the VCS and NES).
4. **The graphics and audio decode backends** — `graphics()` and
   `channel_waves()` are the family's per-vblank decode into the shared
   `GraphicsView` / `ChannelWave` vocabulary.

And a few surfaces are still Game Boy-shaped, quarantined by no-op defaults
until a second family grows the equivalent:

- **GB types ride a few seam signatures** — `WatchCondition`-shaped watch
  methods, `SymbolTable` / `Symbol` label editing, and `CdlWindow`
  (`cdl_window`) are GB-flavoured on `SystemDebugger`, plus the boot-ROM and
  serial-link fields on `MediaLoad`. Generalize each when a second family grows
  the backend.
- **Presentation details** — the GUI's `ScreenView` carries GB palette/SGB
  fields beside the indexed path, and the library's bundled catalogue and
  homebrew browser are Game Boy data flows. Mostly data, not code shape.
- **16-bit addressing assumptions** — breakpoints and `RunningStatus.pc/sp`
  cross the seam as `u32` but every current core masks to a 16-bit bus. Fine
  for every current family; widen when a 32-bit-bus system arrives.

## Quality bars

- **Accuracy oracles in CI from day one.** The GB, GBC, and VCS suites all
  fully pass; the gate for any change is a fully-passing suite. A new core lands
  its own accuracy tests (screenshot or trace parity against its ground-truth
  tier) with it.
- **Round-trip gates for the state story.** A save-state round-trip test and a
  record→replay frame-hash gate are the accuracy bar for the schema work (see
  the `save_state` and `recording` integration tests).
- **Suite-green is the merge gate**, per the per-core methodology docs; ANY
  failure is a regression.

## Checklist for a new family

1. **Core crate** with the console type (hardware-model quality bar applies),
   plus a `crates/<crate>/AGENTS.md` methodology doc (ground-truth hierarchy,
   resources, timing model) and one routing row in the root `AGENTS.md`
   *Per-core methodology* table.
2. **The seam impl** — either a `SteppingSystem` impl for a simple stepping
   core, or hand-written `SystemConsole` + `SystemDebugger` impls where the core
   has its own debugger backend. Registering the core in `missingno-debugger`'s
   factory (`factory.rs`) gives it the whole headless server.
3. **`video_out`** returning the right `DisplayTechnology`, and a palette table
   (or RGBA-producing frame) plus the family's reading of the shared control ids.
4. **The state schema** — a `SystemStateSchema`, `state_schema` / `read_state`,
   and the `save_state` / `load_state` boundary bridge. Save states, traces, and
   recordings follow.
5. **The debugger surfaces** you have — `sidebar_sections`, `instruction_set`,
   the graphics/audio capture hooks — each of which lights up its pane and tool.
6. **Frontend registration** — a `FamilyDescriptor` in `FAMILIES`
   (`app/system/mod.rs`) with its `Platform` variant, a `pane_family()`, and a
   default layout. Dialogs, loading, library scanning, badges, the bindings UI,
   and the trace CLI follow from the table.
7. **Convert audio to 44.1 kHz** on the family's side of the seam.
8. **Accuracy and round-trip tests** committed with the core.
