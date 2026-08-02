/// DFF register cell: holds output and an optional pending value resolved after
/// `commit_in` ticks (1 = the next tick — the default mid-Mode-3 write).
pub struct DffLatch {
    pub(super) output: u8,
    pub(super) pending: Option<u8>,
    commit_in: u8,
}

impl DffLatch {
    pub(super) fn new(initial: u8) -> Self {
        Self {
            output: initial,
            pending: None,
            commit_in: 0,
        }
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    /// Models the dlatch_ee transparency window between write() and the next tick().
    pub fn pending(&self) -> Option<u8> {
        self.pending
    }

    /// Value a combinational reader sees while a staged write is still transparent —
    /// the staged value if present, else the committed output.
    pub fn live(&self) -> u8 {
        self.pending.unwrap_or(self.output)
    }

    /// Returns true if a pending value was captured to output.
    pub(super) fn tick(&mut self) -> bool {
        if self.pending.is_some() {
            self.commit_in = self.commit_in.saturating_sub(1);
            if self.commit_in == 0 {
                self.output = self.pending.take().unwrap();
                return true;
            }
        }
        false
    }

    /// Mode 3 write: pending until next fall.
    pub(super) fn write(&mut self, new_value: u8) {
        self.pending = Some(new_value);
        self.commit_in = 1;
    }

    /// Mode 3 write that the PPU samples `falls` falls late (CGB register-write lag).
    pub(super) fn write_delayed(&mut self, new_value: u8, falls: u8) {
        self.pending = Some(new_value);
        self.commit_in = falls.max(1);
    }

    pub(super) fn write_immediate(&mut self, new_value: u8) {
        self.output = new_value;
        self.pending = None;
        self.commit_in = 0;
    }

    /// CGB register-crossing write: `falls` falls late when `falls > 0`, else the
    /// DMG combinational path (immediate).
    pub(super) fn write_crossing(&mut self, new_value: u8, falls: u8) {
        if falls > 0 {
            self.write_delayed(new_value, falls);
        } else {
            self.write_immediate(new_value);
        }
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.commit_in = 0;
    }
}

/// Single-bit DFF: bool analogue of `DffLatch`. `write` drives the D input,
/// `tick` captures D→Q on the clock edge, `output` reads Q. Edge detection
/// derives from `output()` around `tick()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DffBit {
    pending: bool,
    output: bool,
}

impl DffBit {
    pub(super) fn new(pending: bool, output: bool) -> Self {
        Self { pending, output }
    }

    pub fn output(&self) -> bool {
        self.output
    }

    /// The transparent D input a combinational reader sees before the next tick().
    pub(super) fn pending(&self) -> bool {
        self.pending
    }

    /// Drive the D input.
    pub(super) fn write(&mut self, d: bool) {
        self.pending = d;
    }

    /// Capture D→Q; returns the newly latched output.
    pub(super) fn tick(&mut self) -> bool {
        self.output = self.pending;
        self.output
    }
}

/// Combinational NOR-latch (cross-coupled NOR pair; no clock).
/// Use for RYDY, PYNU, REJO, XYMU, WUSA. Use `DffLatch` for clocked DFFs.
pub struct NorLatch {
    output: bool,
}

impl NorLatch {
    pub(super) fn new(initial: bool) -> Self {
        Self { output: initial }
    }

    pub fn output(&self) -> bool {
        self.output
    }

    pub(super) fn set(&mut self) {
        self.output = true;
    }

    pub(super) fn clear(&mut self) {
        self.output = false;
    }
}
