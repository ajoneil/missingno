/// DFF register cell: holds output and an optional pending value resolved on the next tick.
pub struct DffLatch {
    pub(super) output: u8,
    pub(super) pending: Option<u8>,
}

impl DffLatch {
    pub(super) fn new(initial: u8) -> Self {
        Self {
            output: initial,
            pending: None,
        }
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    /// Models the dlatch_ee transparency window between write() and the next tick().
    pub fn pending(&self) -> Option<u8> {
        self.pending
    }

    /// Returns true if a pending value was captured to output.
    pub(super) fn tick(&mut self) -> bool {
        if let Some(value) = self.pending.take() {
            self.output = value;
            true
        } else {
            false
        }
    }

    /// Mode 3 write: pending until next fall.
    pub(super) fn write(&mut self, new_value: u8) {
        self.pending = Some(new_value);
    }

    pub(super) fn write_immediate(&mut self, new_value: u8) {
        self.output = new_value;
        self.pending = None;
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
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
