//! The shared CPU data bus and the access-trace primitives the
//! debugger uses to observe traffic.

/// The SoC's shared CPU data bus (`cpu_port_d[7:0]`) — the wire
/// driven by whichever peripheral's tri-state driver is enabled.
/// The CPU latches the bus at `data_phase_n↑` near the end of
/// T-cycle 3 of a read M-cycle.
pub struct CpuBus {
    /// Current `cpu_port_d[7:0]` value. Driven at T-cycle 2; latched
    /// by the CPU at end of M-cycle.
    pub data: u8,
    activity: Activity,
}

/// What the CPU is doing on the bus this M-cycle. The CPU asserts
/// either `cpu_rd` or `cpu_wr` per M-cycle, never both.
enum Activity {
    Idle,
    /// Peripheral drives the bus at T-cycle 2; CPU latches at end of
    /// T-cycle 3. `applied` flips true once the T-cycle 2 drive has
    /// fired.
    Read { address: u16, applied: bool },
    /// CPU drives the bus at CUPA-rising (T-cycle 2). Memory commits
    /// at fall of T-cycle 3 / CUPA-falling.
    Write {
        address: u16,
        applied: bool,
        /// OAM/VRAM lock at CUPA-rising. Combined with `locked_at_mid`
        /// and the commit-time sample (AND): the write lands if any
        /// of the three is unlocked. `None` for non-OAM/VRAM addresses.
        locked_at_snapshot: Option<bool>,
        /// OAM/VRAM lock at mid-CUPA — catches the AJUJ-glitch window
        /// where mode-2 ends mid-strobe and the rendering deferral
        /// makes the bus appear unlocked.
        locked_at_mid: Option<bool>,
    },
}

impl CpuBus {
    pub(crate) fn new() -> Self {
        Self {
            data: 0xFF,
            activity: Activity::Idle,
        }
    }

    /// Clear at the M-cycle boundary.
    pub(crate) fn clear_activity(&mut self) {
        self.activity = Activity::Idle;
    }

    pub(crate) fn stage_read(&mut self, address: u16) {
        self.activity = Activity::Read {
            address,
            applied: false,
        };
    }

    pub(crate) fn stage_write(&mut self, address: u16) {
        self.activity = Activity::Write {
            address,
            applied: false,
            locked_at_snapshot: None,
            locked_at_mid: None,
        };
    }

    /// Address of a pending (unapplied) read this M-cycle.
    pub(crate) fn pending_read(&self) -> Option<u16> {
        match self.activity {
            Activity::Read {
                address,
                applied: false,
            } => Some(address),
            _ => None,
        }
    }

    /// Address of a pending (unapplied) write this M-cycle.
    pub(crate) fn pending_write(&self) -> Option<u16> {
        match self.activity {
            Activity::Write {
                address,
                applied: false,
                ..
            } => Some(address),
            _ => None,
        }
    }

    /// Address of an applied write whose mid-CUPA lock sample has
    /// not yet been recorded.
    pub(crate) fn mid_sample_pending(&self) -> Option<u16> {
        match self.activity {
            Activity::Write {
                address,
                applied: true,
                locked_at_mid: None,
                ..
            } => Some(address),
            _ => None,
        }
    }

    /// Drive the bus and mark the staged read/write as applied.
    pub(crate) fn drive(&mut self, value: u8) {
        self.data = value;
        match &mut self.activity {
            Activity::Read { applied, .. } | Activity::Write { applied, .. } => {
                *applied = true;
            }
            Activity::Idle => {}
        }
    }

    /// Record the OAM/VRAM lock at CUPA-rising. No-op outside writes.
    pub(crate) fn record_snapshot_lock(&mut self, lock: Option<bool>) {
        if let Activity::Write {
            locked_at_snapshot, ..
        } = &mut self.activity
        {
            *locked_at_snapshot = lock;
        }
    }

    /// Record the OAM/VRAM lock at mid-CUPA. No-op outside writes.
    pub(crate) fn record_mid_lock(&mut self, lock: Option<bool>) {
        if let Activity::Write { locked_at_mid, .. } = &mut self.activity {
            *locked_at_mid = lock;
        }
    }

    /// AJUJ-window lock samples for a staged write: `(snapshot, mid)`.
    /// `(None, None)` outside writes.
    pub(crate) fn write_lock_samples(&self) -> (Option<bool>, Option<bool>) {
        match self.activity {
            Activity::Write {
                locked_at_snapshot,
                locked_at_mid,
                ..
            } => (locked_at_snapshot, locked_at_mid),
            _ => (None, None),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusAccessKind {
    Read,
    Write,
    DmaRead,
    DmaWrite,
}

#[derive(Clone, Copy, Debug)]
pub struct BusAccess {
    pub address: u16,
    pub value: u8,
    pub kind: BusAccessKind,
}

/// Optional recording of every CPU/DMA bus access during a step.
/// Allocation-free unless `enable` is called.
pub struct BusTrace {
    entries: Option<Vec<BusAccess>>,
}

impl BusTrace {
    pub(crate) fn new() -> Self {
        Self { entries: None }
    }

    pub(crate) fn enable(&mut self) {
        self.entries = Some(Vec::new());
    }

    pub(crate) fn record(&mut self, access: BusAccess) {
        if let Some(entries) = &mut self.entries {
            entries.push(access);
        }
    }

    /// Drain the accumulated trace and disable recording.
    pub(crate) fn take(&mut self) -> Vec<BusAccess> {
        self.entries.take().unwrap_or_default()
    }
}
