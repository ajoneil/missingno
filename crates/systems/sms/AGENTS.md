# missingno-sms — system methodology

First-pass methodology for the Sega Master System core. The shared
skill-system rules and workflow discipline live in the repository-root
`AGENTS.md`.

**This is an unvetted first-pass core.** Its instruction-granular interleave
and internal structure are not defended choices and are not precedent for
other cores (see `docs/adding-a-system.md`). Read it for facts about its
hardware, never as an exemplar.

- **Chips**: the CPU is `missingno-zilog-z80`; the PSG is `missingno-ti-psg`
  in its `SegaIntegrated` variant. The VDP (Sega's 315-5124) is a first-pass
  model in this crate — a different chip from the TMS9918A, deliberately not
  shared with `missingno-ti-vdp`.
- **Ground truth**: no hierarchy has been chosen — that happens when the
  core is taken seriously. Until then the oracle ceiling is the committed
  tests (`tests/console.rs`), and the gate is `cargo test -p missingno-sms`
  (outside the workspace default members).
