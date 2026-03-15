/// Fetch-done cascade: LYRY → NYKA → PORY → PYGO → POKY.
///
/// A DFF chain that propagates the fetcher-idle signal (LYRY) through four
/// stages, adding pipeline delay before the pixel clock enables. Not a
/// processing block — just a small state machine you clock and query.
///
/// - NYKA (DFF17, falling/ALET): captures live LYRY (combinational)
/// - PORY (DFF17, rising/MYVO): captures NYKA
/// - PYGO (DFF17, falling/ALET): captures PORY
/// - POKY (NOR latch, falling): fires from PYGO
///
/// Consumers read DFF state via accessors:
/// - `poky()` → TYFA (pixel clock enable)
/// - `pygo()` → sprite wait exit, TAVE guard, window trigger gate
/// - `pory()` → RYDY clear
/// - `nyka()` + `pory()` → TAVE preload
pub(super) struct FetchCascade {
    /// Previous falling phase's LYRY value. Models DFF17 `state_old` read.
    lyry_prev: bool,
    /// NYKA_FETCH_DONEp_evn: latches on falling edge (ALET).
    nyka: bool,
    /// PORY_FETCH_DONEp_odd: latches on rising edge (MYVO).
    pory: bool,
    /// PYGO_FETCH_DONEp_evn: latches on falling edge (ALET).
    pygo: bool,
    /// POKY NOR latch: fires from PYGO on falling edge.
    poky: bool,
}

impl FetchCascade {
    pub(super) fn new() -> Self {
        FetchCascade {
            lyry_prev: false,
            nyka: false,
            pory: false,
            pygo: false,
            poky: false,
        }
    }

    /// Falling edge: clock NYKA from live LYRY, update lyry_prev,
    /// clock PYGO from PORY, fire POKY NOR from PYGO.
    pub(super) fn fall(&mut self, lyry: bool) {
        // NYKA captures live LYRY — LYRY is combinational and updates
        // in the same falling phase the fetcher reaches Idle.
        if lyry && !self.nyka {
            self.nyka = true;
        }

        // Update lyry_prev for next falling phase.
        self.lyry_prev = lyry;

        // PYGO captures PORY on falling edge (ALET clock).
        if self.pory && !self.pygo {
            self.pygo = true;
        }

        // POKY NOR latch fires on falling, reading the just-updated PYGO.
        if self.pygo && !self.poky {
            self.poky = true;
        }
    }

    /// Rising edge: clock PORY from NYKA.
    pub(super) fn rise(&mut self) {
        if self.nyka && !self.pory {
            self.pory = true;
        }
    }

    /// Scanline reset: clear all DFFs.
    pub(super) fn reset(&mut self) {
        self.lyry_prev = false;
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.poky = false;
    }

    /// NAFY window-trigger reset: clear NYKA, PORY, and lyry_prev.
    /// PYGO and POKY are not reset by window triggers.
    pub(super) fn reset_window(&mut self) {
        self.nyka = false;
        self.pory = false;
        self.lyry_prev = false;
    }

    /// Clear lyry_prev so the cascade sees a reset fetcher state.
    /// Used when a sprite fetch resets the fetcher counter (SEKO).
    pub(super) fn clear_lyry(&mut self) {
        self.lyry_prev = false;
    }

    pub(super) fn lyry_prev(&self) -> bool {
        self.lyry_prev
    }
    pub(super) fn nyka(&self) -> bool {
        self.nyka
    }
    pub(super) fn pory(&self) -> bool {
        self.pory
    }
    pub(super) fn pygo(&self) -> bool {
        self.pygo
    }
    pub(super) fn poky(&self) -> bool {
        self.poky
    }
}
