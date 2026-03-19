/// Fetch-done cascade: LYRY → NYKA → PORY → PYGO → POKY.
///
/// A DFF chain that propagates the fetcher-idle signal (LYRY) through four
/// stages, adding pipeline delay before the pixel clock enables. Not a
/// processing block — just a small state machine you clock and query.
///
/// - NYKA (DFF17, falling/ALET): captures LYRY
/// - PORY (DFF17, rising/MYVO): captures NYKA
/// - PYGO (DFF17, falling/ALET): captures PORY
/// - POKY (NOR latch, falling): fires from PYGO
///
/// On hardware, LYRY fires on ODD (rising) and NYKA captures on EVEN
/// (falling). The clock edge separation provides 1 half-phase of
/// pipeline delay. NYKA's input (`lyry`) is latched at the end of
/// each falling phase, so the next `fall()` reads the previous
/// phase's value — standard DFF behavior.
///
/// Consumers read DFF state via accessors:
/// - `poky()` → TYFA (pixel clock enable)
/// - `pygo()` → sprite wait exit, TAVE guard, window trigger gate
/// - `pory()` → RYDY clear
/// - `nyka()` + `pory()` → TAVE preload
pub(super) struct FetchCascade {
    /// LYRY DFF input: latched at the end of each falling phase so that
    /// NYKA reads the value from the previous phase on the next fall.
    lyry: bool,
    /// NYKA_FETCH_DONEp_evn: DFF17, latches on falling edge (ALET).
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
            lyry: false,
            nyka: false,
            pory: false,
            pygo: false,
            poky: false,
        }
    }

    /// Falling edge: clock NYKA from LYRY, clock PYGO from PORY,
    /// fire POKY NOR from PYGO.
    pub(super) fn fall(&mut self, lyry: bool) {
        // NYKA DFF17: captures LYRY on falling edge (ALET clock).
        // Read self.lyry (latched from the previous phase), then
        // update self.lyry from the live value for the next cycle.
        if self.lyry && !self.nyka {
            self.nyka = true;
        }
        self.lyry = lyry;

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
        self.lyry = false;
        self.nyka = false;
        self.pory = false;
        self.pygo = false;
        self.poky = false;
    }

    /// NAFY window-trigger reset: clear NYKA and PORY.
    /// PYGO and POKY are not reset by window triggers.
    pub(super) fn reset_window(&mut self) {
        self.lyry = false;
        self.nyka = false;
        self.pory = false;
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
