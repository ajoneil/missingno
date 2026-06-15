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

    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.commit_in = 0;
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
