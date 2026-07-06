# Adding an emulated system

Missingno is built so a second console family can arrive without disturbing the
first. This document maps the seams a new system plugs into, what each one
demands, and which parts of the frontend are still Game Boy-shaped and would
need widening first. It reflects the code as of 2026-07; trust the seams named
here, but verify signatures against the source before building on them.

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

## What a new core crate provides

The core crates are hardware models first; the emulation philosophy in
AGENTS.md (edge-level modelling, code-as-documentation, hardware as the source
of truth) applies to any new core. Structurally, the frontend needs a console
type that can:

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
called from `app/load.rs`; the first non-GB system promotes that call into a
detection step — by extension or header sniffing — that picks a family
factory).

## Honest inventory: what is still Game Boy-shaped

These are known, deliberate residues (kept until a second family exists to
justify the abstraction). Widening them is part of the first new system's
work, in roughly this order:

1. **`ScreenDisplay` / `ScreenView`** (`app/screen.rs`) — the frame types are
   GB render formats (DMG indexed + user palette, SGB attributed, CGB RGBA)
   with UI-side palette resolution. Path: an erased frame type that resolves
   to RGBA given view preferences, per family.
2. **`joypad::Button` in the seam traits** — the input vocabulary is GB's
   eight buttons, and the settings bindings model (`Action::Gb*` in
   `app/settings/mod.rs`) matches. Path: per-system button sets; needs a
   settings-file migration (see the sprint plan docs under
   `receipts/fable-sprint-2026-07/plans/`).
3. **`Cartridge` in the seam traits** — load/detail/save flows consume
   `missingno_gb::cartridge::Cartridge` directly. Path: narrow the seam to
   the operations actually used (title, battery presence, save bytes).
4. **The debugger inspection vocabulary** (`app/debugger/inspect.rs`) —
   `InspectSource` exposes GB CPU/PPU/VRAM views, and the pane set
   (`PANE_REGISTRY` in `app/debugger/panes.rs`) is the GB set. Path: the pane
   registry becomes per-family, each family bringing panes that understand its
   own inspection types. The registry + `Pane` trait are already shaped for
   this — panes are self-contained trait objects registered in one table.
5. **The library/gamedb** — game identification is SHA1-based and
   platform-tagged (`GameEntry.platform` is already a string field);
   the bundled catalogue is Game Boy titles. Mostly data, not code shape.

## Checklist for the first new family

1. Core crate with the console type (hardware-model quality bar applies).
2. `app/system/<family>.rs`: `SystemConsole` + `SystemDebugger` impls,
   media-metadata constants, factory.
3. Widen `ScreenDisplay` (item 1 above) and the input vocabulary (item 2).
4. ROM detection → family factory dispatch in the load path.
5. Family pane set if it ships a debugger; otherwise implement
   `SystemDebugger` minimally (stepping + breakpoints) and grow it.
6. Library: extension/platform registration so scanning and dialogs see the
   new media.
