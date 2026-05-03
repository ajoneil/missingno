// --- Fine scroll (ROXY pixel clock gate) ---

/// ROXY NOR latch state. On hardware, ROXY gates the pixel clock
/// (SACU = or2(SEGU, ROXY)) until the fine scroll counter matches
/// SCX & 7. SET between lines (PAHA_RENDERINGn), RESET on fine
/// scroll match (POVA fine match trigger). One-shot per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Roxy {
    /// ROXY=1: pixel clock gated. The fine counter is still counting
    /// toward the SCX & 7 target.
    Gating,
    /// ROXY=0: pixel clock active. Fine scroll discard is complete
    /// for this line.
    Done,
}

/// Hardware fine scroll counter (RYKU/ROGA/RUBU) and pixel clock
/// gate (ROXY). The ROXY latch gates the pixel clock (SACU) until
/// the counter matches SCX & 7, implementing sub-tile fine scrolling.
///
/// # Collapsed match-decode signals (not modelled as explicit state)
///
/// The fine-scroll subsystem decodes the counter-matches-SCX condition
/// through a multi-gate chain that this module collapses to a single
/// numeric comparison (`count == scx & 7`). Collapsed signals:
///
/// - Per-bit compare: `SUHA = XNOR(ff43_d0, RYKU)`,
///   `SYBY = XNOR(ff43_d1, ROGA)`, `SOZU = XNOR(ff43_d2, RUBU)` — three
///   XNORs each firing when one SCX bit matches the corresponding counter
///   bit.
/// - Drain-gated aggregate: `RONE = NAND4(ROXY, SUHA, SYBY, SOZU)` — the
///   NAND4 whose ROXY input is the self-termination gate (ROXY=0 after
///   first POVA forces RONE=1 forces POHU=0, de-asserting the match
///   pipeline without needing a separate "done" flag).
/// - Comparator output: `POHU = NOT(RONE) = AND4(ROXY, SUHA, SYBY, SOZU)`.
///   Fires during ROXY-gated startup when all three counter bits match
///   SCX[0:2].
///
/// Collapsed cascade: `SCX bits + counter bits → SUHA/SYBY/SOZU → RONE → POHU`.
///
/// The emulator's `compare_falling(scx)` computes `pohu = count == (scx & 7)`
/// directly, dropping the per-bit XNOR decomposition and the NAND4
/// aggregate. Observation-equivalence at the POHU-field output boundary is
/// preserved because (a) the numeric equality is definitionally the
/// AND4 of per-bit XNORs; (b) POHU's ROXY drain-gate is implicitly
/// respected because the emulator only consumes the `pohu` field while
/// `roxy == Roxy::Gating` (hardware POHU during ROXY=0 is always 0 via
/// RONE; emulator bypasses POHU reads during Roxy::Done).
///
/// Counter self-stop (ROZE): `ROZE = NAND3(RUBU, ROGA, RYKU)` is the
/// count-at-7 self-stop feeding PECU. Emulator collapses this via
/// `tick()`'s `if self.count < 7` guard — observation-equivalent at the
/// counter-bits output boundary.
///
/// # Honest-abstraction synthesis
///
/// Fine-scroll collapse decisions are split into two categories:
///
/// **Observation-equivalent collapses** (internals abstracted; all
/// visible boundaries reproduce hardware):
///
/// - Match decode (SUHA/SYBY/SOZU/RONE → POHU) collapsed into the
///   numeric `count == scx & 7` compare with ROXY gating implicit in
///   the consumer pattern (emulator only reads `pohu` while
///   `roxy == Roxy::Gating`).
/// - ROZE counter self-stop collapsed into the `count < 7` tick guard.
/// - PASO reset collapsed into `FineScroll::new()` at scanline reset
///   and `reset_counter()` at TEVO / window-trigger events.
/// - ROXY clear transition modelled as a one-shot state change
///   (`Roxy::Gating → Roxy::Done`) tracking hardware's NOR-latch set-
///   once-per-scanline semantics; re-arming only at next scanline via
///   struct reinitialisation.
///
/// **Flagged divergences** (known gaps, deferred to adjacent subsystem
/// alignment; see `nyze` field doc-comment for detail):
///
/// - POVA pulse width: hardware level-AND (~1 dot) vs emulator
///   rising-edge (single tick). Benign at the ROXY-clear consumer;
///   pulse-shape differs for level-sensitive consumers (none in
///   emulator currently).
/// - Missing POVA contribution to SEMU: hardware
///   `SEMU = OR2(TOBA, POVA)` generates an extra CP rising edge per
///   Mode 3 scanline; emulator computes TOBA-only pixel output and
///   doesn't model SEMU / rypo / cp_pad. The extra CP edge is absent.
///
/// Both flagged divergences belong to LCD-output-pipeline territory
/// (SEMU / rypo / cp_pad primary in the output pin drivers). Resolution
/// awaits the LCD-output alignment arc (tracked in alignment-log's
/// "Deferred follow-on arcs" section) where cp_pad waveform modelling
/// would make SEMU's POVA term a real consumer.
pub(in crate::ppu) struct FineScroll {
    /// 3-bit counter (0–7).
    pub(in crate::ppu) count: u8,
    /// ROXY NOR latch — gates SACU until fine scroll match fires.
    roxy: Roxy,
    /// NYZE DFF — stores previous PUXA to support the emulator's
    /// rising-edge detector for POVA. Hardware formula is level-AND:
    /// `POVA = AND2(NYZE, PUXA)` where NYZE captures PUXA on MOXE
    /// rising (one half-cycle lag), giving a pulse that starts when
    /// NYZE catches up to PUXA=1 and ends when PUXA drops to 0 via
    /// POHU self-termination (ROXY drain-gate through RONE). Emulator
    /// instead computes `pova = puxa && !nyze` — a rising-edge detector
    /// that fires for a single tick.
    ///
    /// **Known divergences** (not observation-equivalent at all
    /// boundaries):
    ///
    /// 1. **POVA pulse width**: hardware produces a roughly-one-dot
    ///    pulse (duration = MOXE half-cycle + propagation until PUXA
    ///    drops via POHU drain); emulator produces a single-tick pulse.
    ///    At the ROXY-clear consumer boundary this is benign — ROXY's
    ///    NOR-latch only needs one rising edge to clear — but the
    ///    pulse-shape itself differs.
    /// 2. **Missing POVA contribution to SEMU**: hardware wires
    ///    `SEMU = OR2(TOBA, POVA)` so each Mode 3 scanline has one
    ///    extra CP rising edge from the POVA pulse (in addition to the
    ///    per-pixel TOBA edges starting at PX=9). Emulator computes
    ///    TOBA directly for pixel output with no POVA contribution; the
    ///    extra CP edge is absent. The emulator does not model the
    ///    cp_pad waveform (no rypo/SEMU implementation), so no current
    ///    consumer observes the missing edge — but the behaviour is
    ///    absent, not equivalent.
    ///
    /// Both divergences reach `cp_pad` / SEMU / rypo territory in the
    /// LCD output pipeline. Resolution belongs to the LCD-output
    /// alignment work, not the fine-scroll subsystem.
    /// Flagged here as an honest-abstraction boundary, not a claim of
    /// equivalence.
    nyze: bool,
    /// POHU comparator result, computed on PPU clock rise.
    /// PUXA captures this on the next PPU clock fall (when ROXO fires).
    pohu: bool,
}

impl FineScroll {
    pub(in crate::ppu) fn new() -> Self {
        Self {
            count: 0,
            roxy: Roxy::Gating,
            nyze: false,
            pohu: false,
        }
    }

    /// Whether the pixel clock is active (SACU ungated).
    pub(in crate::ppu) fn pixel_clock_active(&self) -> bool {
        self.roxy == Roxy::Done
    }

    /// Advance the fine counter by one dot (PECU clock).
    /// Self-stops at 7 (ROZE gate: nand3(CNT2, CNT1, CNT0)).
    pub(in crate::ppu) fn tick(&mut self) {
        if self.count < 7 {
            self.count += 1;
        }
    }

    /// TEVO → PASO: reset the fine counter to 0.
    pub(in crate::ppu) fn reset_counter(&mut self) {
        self.count = 0;
    }

    /// Falling phase: compute POHU comparator result from current count.
    /// On hardware, POHU is combinational (count == SCX & 7), and ROXO
    /// captures it into PUXA on the falling edge. We store it for the
    /// next rising phase's capture step.
    pub(in crate::ppu) fn compare_falling(&mut self, scx: u8) {
        self.pohu = self.count == (scx & 7);
    }

    /// Rising phase: capture PUXA from stored POHU, edge-detect POVA,
    /// and clear ROXY if matched.
    ///
    /// Only call when TYFA is active (ROXO fires).
    /// Returns POVA = AND2(PUXA, !NYZE).
    pub(in crate::ppu) fn capture_rising(&mut self) -> bool {
        let puxa = self.pohu;
        let pova = puxa && !self.nyze;
        self.nyze = puxa;

        if self.roxy == Roxy::Gating && puxa {
            self.roxy = Roxy::Done;
        }

        pova
    }

    /// Reset for window trigger — counter resets, gating clears
    /// (window has no fine scroll).
    pub(in crate::ppu) fn reset_for_window(&mut self) {
        self.count = 0;
        self.roxy = Roxy::Done;
        self.pohu = false;
    }
}
