# Adding an emulated system

Missingno is built so a second console family can arrive without disturbing the
first. This document maps the seams a new system plugs into, what each one
demands, and which parts of the frontend are still Game Boy-shaped and would
need widening first. Trust the seams named here, but verify signatures
against the source before building on them.

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
- **A new family entirely** (Game Gear, NES, …) — a new core crate plus a new
  frontend submodule. The rest of this document is about this axis.

## The accuracy philosophy, per core

Use the available evidence to reach the highest accuracy possible — **and the
mechanism that achieves it is a per-core decision**. The
Game Boy runs a fused-T-cycle lockstep because gate-level ground truth
(dmg-sim, the netlist) makes sub-cycle ordering verifiable and the suites
demand it. A system whose best evidence is test-ROM-granular must not pay
for — or claim — fidelity nobody can check; its design doc picks the
internal quantum (dot, color clock, master-cycle slice, instruction-granular
catch-up) and defends it against that system's ground-truth tier.

What is NOT per-core is the contract with the frontend, debugger, tests, and
tracing. Any internal mechanism must provide:

1. **Determinism** — same ROM + inputs → bit-exact execution (replay,
   tracing, and bisection depend on it).
2. **Instruction-boundary stepping** for the debugger's step/breakpoints.
3. **On-demand bus observability** without behaviour change (watchpoints,
   code/data logging, trace capture — the `BusTrace` pattern).
4. **Side-effect-free inspection reads** (disassembly, memory panes).
5. **A cheap owned per-frame snapshot** for the running debugger view.
6. **Budgeted frame stepping** with a stall guard, frames as data.
7. **Committable test oracles in CI from day one** — accuracy claims live
   in tests.

## What a new core crate provides

Structurally, the frontend needs a console type that can:

- construct from ROM bytes plus optional save data and boot ROM
- step, reporting per-step: cycles consumed, whether a display frame
  completed, whether battery-backed save memory was dirtied
  (see `StepResult` in `missingno-gb`)
- expose the completed frame, drain stereo `f32` audio samples, accept
  button press/release, export save memory
- optionally: a debugger wrapper with breakpoints/watchpoints and
  side-effect-free memory reads (see `missingno_gb::debugger`)

`missingno-gb` is the worked example: `Console<M>` in `lib.rs`, the step loop
in `execute.rs`, the debugger backend under `debugger/`.

## The frontend seam: `crates/missingno/src/app/system/`

The app shell (library, emulator screen, emulation thread, debugger UI) drives
consoles only through two object-safe traits in `app/system/mod.rs`:

- **`SystemConsole`** — run a frame within a step budget, reset, input, audio,
  produce a `ScreenDisplay`, capture screenshots, expose save data, report its
  wall-clock `frame_interval()` for the pacing loop, and convert
  `into_debugger()`.
- **`SystemDebugger`** — stepping (instruction / over / frame), breakpoints and
  watchpoints, debug symbols, an `inspect()` surface for the paused panes, an
  owned `snapshot()` the running view renders from, and `into_console()`.

The Game Boy family implements both once, generically over its `Model` seam, in
`app/system/gb.rs`. That file is the template: media metadata (file-dialog
filters, extensions, platform name) lives there as named constants, and
`create_console` is the family's single registration point.

A new family means: one new submodule implementing the two traits, and factory
dispatch wherever ROMs are identified (today `create_console` is GB-only and
called from `app/load.rs`, which detects the family by extension and
header shape and picks its factory).

The seams a second family exercises are in place (the Atari 2600 is the
worked second consumer, feature-gated):

- **Frames**: `ScreenDisplay::Indexed` carries per-frame dimensions,
  palette-index pixels, and the family's palette table, resolved to RGBA
  at draw time. The GB render formats remain their own variants.
- **Input**: the seam takes `set_control(ControlId, ControlInput)` —
  family-interpreted ids (0-7 mirror the GB button order so the bindings
  pipeline translates numerically; 8 and up are analog and
  family-specific). Families read the ids as their own hardware.
- **Media**: the seam carries `game_title` and `battery_save` only; how a
  family serializes saves is its own concern.
- **Audio**: the contract is 44.1 kHz stereo `f32`; families convert from
  their native rate on their own side.
- **Debugger**: `Inspection` family-erases the inspection surface (each
  family exposes a typed accessor); pane registries, default layouts, and
  layout sidecars are family-provided; `into_debugger` is fallible so a
  family without a debugger backend falls back to plain emulation;
  `RunningStatus` words its own video summary.

## Honest inventory: what is still Game Boy-shaped

1. **The bindings/settings model** — the biggest remaining piece. Input
   bindings speak `Action::Gb*` → `joypad::Button` and translate
   numerically at the seam. Path: families publish labelled control
   tables; the settings model and bindings UI become per-family (needs a
   settings-file migration, and it is UI-design-heavy).
2. **Ancillary debugger types in the seam** — `WatchCondition`,
   `SymbolTable`/`Symbol`, `CdlWindow`, and `SerialLink` are GB types;
   non-GB families accept and report empty. Generalize when a second
   family grows real watchpoints/symbols — the natural moment is its
   bus-observability work.
3. **Presentation details** — `IndexedFrame` has no pixel-aspect hint
   (some systems' pixels are non-square); `ScreenView` carries GB
   palette/SGB fields beside the indexed path; the screenshot gallery
   sizes thumbnails from the GB frame dimensions (the captures themselves
   carry their own).
4. **Family registration is by hand** — extensions, platform names, and
   detection live as per-family constants with call-site branches in
   `app/load.rs` and the library scanner. Fine at two families; a family
   descriptor table earns its keep at the third.
5. **The library/gamedb** — game identification is SHA1-based and
   platform-tagged (`GameEntry.platform` is already a string field);
   the bundled catalogue is Game Boy titles. Mostly data, not code shape.

## Checklist for a new family

1. Core crate with the console type (hardware-model quality bar applies).
2. `app/system/<family>.rs`: `SystemConsole` + `SystemDebugger` impls,
   media-metadata constants, factory.
3. Detection branch in `app/load.rs` and extensions in the library
   scanner and file dialog.
4. A palette table (or RGBA-producing path) for `ScreenDisplay::Indexed`,
   and the family's reading of the shared control ids.
5. Family pane set + default layout if it ships a debugger; otherwise
   return the console from `into_debugger` and the shell falls back to
   plain emulation.
6. Convert audio to 44.1 kHz on the family's side of the seam.
