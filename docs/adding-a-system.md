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
- **`SystemDebugger`** — stepping (instruction / over / frame), breakpoints,
  an `inspect()` surface for the paused panes, an owned `snapshot()` the
  running view renders from, and `into_console()`. Watchpoints, debug
  symbols, code/data logging, trace capture, and battery saves have default
  no-op implementations — a family implements only the backends it has.

The Game Boy family implements both once, generically over its `Model` seam, in
`app/system/gb.rs`. Non-GB families register in the `FAMILIES` descriptor
table in `app/system/mod.rs` — platform name, extensions, a `detect`
predicate, and a console factory. The file dialog, ROM loading, headerless
title detection, and the library scanner all iterate that table; the Game Boy
stays the loader's explicit fallback (its media carries battery saves, boot
ROMs, and the serial link, which attach in its own factory).

The seams several families exercise (VCS, SMS, and NES are the worked
consumers, feature-gated):

- **Frames**: `ScreenDisplay::Indexed` carries per-frame dimensions,
  palette-index pixels, and the family's palette table, resolved to RGBA
  at draw time (`IndexedFrame::blank`, `FrameCapture::from_indexed` are the
  shared helpers). The GB render formats remain their own variants.
- **Input**: the seam takes `set_control(ControlId, ControlInput)` —
  family-interpreted ids (0-7 mirror the GB button order so the bindings
  pipeline translates numerically; 8 and up are analog and
  family-specific). Families read the ids as their own hardware.
- **Media**: the seam carries `game_title` and `battery_save` only; how a
  family serializes saves is its own concern.
- **Audio**: the contract is 44.1 kHz stereo `f32`; families convert from
  their native rate on their own side.
- **Debugger**: `Inspection` family-erases the inspection surface — the GB's
  structured surface goes through `as_gb()`; every other family exposes one
  typed state object through `family_state()` (a `&dyn Any`) that its own
  panes downcast back out of the `PaneContext`. Pane registries, default
  layouts, and layout sidecars are family-provided through `panes::Family`;
  the paused sidebar for non-GB families renders the same `RunningStatus`
  summary as the running view; `into_debugger` is fallible so a family
  without a debugger backend falls back to plain emulation.

## Honest inventory: what is still Game Boy-shaped

1. **The bindings/settings model** — the biggest remaining piece. Input
   bindings speak `Action::Gb*` → `joypad::Button` and translate
   numerically at the seam, so the settings UI labels every family's
   controls with GB names. Path: families publish labelled control
   tables; the settings model and bindings UI become per-family (needs a
   settings-file migration, and it is UI-design-heavy).
2. **GB types ride the seam signatures** — `WatchCondition`,
   `SymbolTable`/`Symbol`, and `CdlWindow` are GB types in `SystemDebugger`
   method signatures. The default implementations quarantine them (non-GB
   families never mention them); generalize the payload types when a second
   family grows real watchpoints/symbols — the natural moment is its
   bus-observability work.
3. **Presentation details** — `IndexedFrame` has no pixel-aspect hint
   (some systems' pixels are non-square — the VCS's are roughly 2:1);
   `ScreenView` carries GB palette/SGB fields beside the indexed path; the
   screenshot gallery sizes thumbnails from the GB frame dimensions (the
   captures themselves carry their own). `capture_frame`'s SGB/palette
   parameters are likewise GB-shaped.
4. **The library/gamedb** — game identification is SHA1-based and
   platform-tagged (`GameEntry.platform` is already a string field);
   the bundled catalogue is Game Boy titles. Mostly data, not code shape.
5. **16-bit addressing** — breakpoints, symbols, and `RunningStatus.pc/sp`
   assume `u16` addresses. Fine for every current family; widen when a
   32-bit-bus system arrives.

## Checklist for a new family

1. Core crate with the console type (hardware-model quality bar applies).
2. `app/system/<family>.rs`: `SystemConsole` + `SystemDebugger` impls,
   media-metadata constants, factory — plus one entry in the `FAMILIES`
   descriptor table in `app/system/mod.rs`. Dialogs, loading, and library
   scanning follow from the table.
3. A palette table (or RGBA-producing path) for `ScreenDisplay::Indexed`,
   and the family's reading of the shared control ids.
4. If it ships a debugger: an inspection-state struct + panes module under
   `app/debugger/<family>.rs` (implement `Inspection::family_state` on the
   state and its snapshot), a `panes::Family` static with its registry and
   default layout, entries in `DebuggerPane` and `PANE_FAMILIES`, and a
   `running_status()` wording its own video summary. Otherwise return the
   console from `into_debugger` and the shell falls back to plain emulation.
5. Convert audio to 44.1 kHz on the family's side of the seam.
