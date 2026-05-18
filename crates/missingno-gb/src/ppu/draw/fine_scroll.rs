/// ROXY NOR-latch: gates SACU until the fine counter matches SCX & 7. One-shot per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Roxy {
    Gating,
    Done,
}

/// RYKU/ROGA/RUBU fine-scroll counter + ROXY pixel-clock gate.
///
/// Match decode (SUHA/SYBY/SOZU → RONE → POHU) is collapsed to `count == scx & 7`.
/// Counter self-stop (ROZE) collapsed into the `count < 7` guard in `tick()`.
///
/// Known divergence: POVA is modelled as a single-tick rising-edge pulse rather than hardware's
/// ~1-dot level-AND; the missing SEMU=OR2(TOBA, POVA) contribution is not wired because cp_pad
/// is not modelled. Benign at the ROXY-clear consumer.
pub(in crate::ppu) struct FineScroll {
    /// 3-bit counter (0–7).
    pub(in crate::ppu) count: u8,
    roxy: Roxy,
    /// Previous PUXA for the rising-edge detector that produces POVA.
    nyze: bool,
    /// POHU comparator result; PUXA captures this on the next fall (ROXO).
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

    pub(in crate::ppu) fn pixel_clock_active(&self) -> bool {
        self.roxy == Roxy::Done
    }

    /// Self-stops at 7 (ROZE gate).
    pub(in crate::ppu) fn tick(&mut self) {
        if self.count < 7 {
            self.count += 1;
        }
    }

    /// TEVO → PASO.
    pub(in crate::ppu) fn reset_counter(&mut self) {
        self.count = 0;
    }

    /// Combinational POHU = (count == SCX & 7); captured into PUXA on the next rise (ROXO).
    pub(in crate::ppu) fn compare_falling(&mut self, scx: u8) {
        self.pohu = self.count == (scx & 7);
    }

    /// Capture PUXA, edge-detect POVA, clear ROXY on match. Caller gates on TYFA.
    pub(in crate::ppu) fn capture_rising(&mut self) -> bool {
        let puxa = self.pohu;
        let pova = puxa && !self.nyze;
        self.nyze = puxa;

        if self.roxy == Roxy::Gating && puxa {
            self.roxy = Roxy::Done;
        }

        pova
    }

    /// Window trigger resets the counter only; ROXY and `pohu` keep their state so the
    /// in-flight pre-window SACU at SCX=0,WX=0 can fire on the MOSU↑ dot.
    pub(in crate::ppu) fn reset_for_window(&mut self) {
        self.count = 0;
    }
}
