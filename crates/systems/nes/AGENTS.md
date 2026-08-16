# missingno-nes — system methodology

First-pass methodology for the Nintendo NES core. The shared skill-system
rules and workflow discipline live in the repository-root `AGENTS.md`.

**This is an unvetted first-pass core.** Its instruction-granular interleave
and internal structure are not defended choices and are not precedent for
other cores (see `docs/adding-a-system.md`). Read it for facts about its
hardware, never as an exemplar.

- **Chips**: the CPU is `missingno-mos-6502` in its decimal-less `nes6502`
  reading; the PPU and APU are first-pass models in this crate. Cartridges
  are iNES NROM only — other mappers are rejected at load.
- **Ground truth**: no hierarchy has been chosen — that happens when the
  core is taken seriously. Until then the oracle ceiling is the committed
  tests (`tests/console.rs`, `tests/trace.rs`), and the gate is
  `cargo test -p missingno-nes` (outside the workspace default members).
