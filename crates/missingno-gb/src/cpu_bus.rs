//! The SoC's shared CPU data bus (`cpu_port_d[7:0]`): its current
//! value, the M-cycle activity tracking that times reads and writes
//! onto it at hardware-accurate dots, and the access-trace
//! primitives the debugger uses to observe traffic.

/// The SoC's shared CPU data bus (`cpu_port_d[7:0]`).
///
/// On hardware, a real wire driven by whichever peripheral's tri-state
/// driver is enabled (PPU registers via `tobe`/`wafu`, work RAM, OAM,
/// VRAM, cartridge, IF/IE, etc.). The CPU latches the bus at
/// `data_phase_n↑` (dot 3.995 of the read M-cycle).
///
/// Driver outputs settle within ~80 ns of the driver enabling at
/// dot 2.005. Subsequent same-M-cycle source-state transitions
/// experience ~340 ns of bus-voltage flux that extends past the
/// CPU's latch edge — the CPU therefore captures the value the
/// driver was stably driving at dot 2.085.
pub struct CpuBus {
    /// Value currently on `cpu_port_d[7:0]`. Updated at dot 2 of each
    /// CPU read/write M-cycle; latched by the CPU at end of M-cycle.
    pub data: u8,
    activity: Activity,
}

/// What the CPU is doing on the bus during the current M-cycle.
///
/// At most one of read/write is in flight at any time: the CPU
/// asserts either `cpu_rd` or `cpu_wr` per M-cycle, never both.
enum Activity {
    Idle,
    /// CPU is reading from `address`. The addressed peripheral's
    /// tri-state driver (`tobe`/`wafu`) enables at dot 2.005 and
    /// drives `cpu_port_d`; the CPU latches at dot 3.995. `applied`
    /// flips true once the dot-2 drive has fired.
    Read { address: u16, applied: bool },
    /// CPU is writing to `address`. The write value is driven onto
    /// the bus at CUPA-rising (rise of dot 2). PPU register latches
    /// are transparent during dots 2-3 and capture at CUPA-falling
    /// (end of dot 3); memory commits at fall() of dot 3.
    Write {
        address: u16,
        applied: bool,
        /// OAM/VRAM lock state at CUPA-rising (rise of dot 2).
        /// Combined with the mid-CUPA and commit-time samples — the
        /// three samples model the AJUJ-high window: a write lands
        /// if AJUJ is high at ANY edge during the CUPA strobe, so
        /// block iff locked at ALL three.
        /// None = non-OAM/VRAM address.
        locked_at_snapshot: Option<bool>,
        /// OAM/VRAM lock state at the mid-CUPA edge (fall of dot 2).
        /// Catches the AJUJ-glitch window when AVAP fires this fall:
        /// BESU clears immediately (mode2↓), begin_rendering defers
        /// to the next rise (mode3↑), so this sample sees
        /// `mode2=0 AND mode3=0` → unlocked. The write then lands
        /// at the straddle even though both snap and commit see
        /// locked state.
        /// None = non-OAM/VRAM address.
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

    /// Clear at the M-cycle boundary (rise of dot 0).
    pub(crate) fn clear_activity(&mut self) {
        self.activity = Activity::Idle;
    }

    /// Stage a read for this M-cycle.
    pub(crate) fn stage_read(&mut self, address: u16) {
        self.activity = Activity::Read {
            address,
            applied: false,
        };
    }

    /// Stage a write for this M-cycle.
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
    /// not yet been recorded. Used at fall of dot 2 to drive the
    /// AJUJ-glitch sampling.
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

    /// Drive the bus with `value` and mark the staged read/write as
    /// applied. On `Idle`, only `data` is updated.
    pub(crate) fn drive(&mut self, value: u8) {
        self.data = value;
        match &mut self.activity {
            Activity::Read { applied, .. } | Activity::Write { applied, .. } => {
                *applied = true;
            }
            Activity::Idle => {}
        }
    }

    /// Record the OAM/VRAM lock state at CUPA-rising (rise of dot 2).
    /// No-op for non-write activity.
    pub(crate) fn record_snapshot_lock(&mut self, lock: Option<bool>) {
        if let Activity::Write {
            locked_at_snapshot, ..
        } = &mut self.activity
        {
            *locked_at_snapshot = lock;
        }
    }

    /// Record the OAM/VRAM lock state at mid-CUPA (fall of dot 2).
    /// No-op for non-write activity.
    pub(crate) fn record_mid_lock(&mut self, lock: Option<bool>) {
        if let Activity::Write { locked_at_mid, .. } = &mut self.activity {
            *locked_at_mid = lock;
        }
    }

    /// AJUJ-window lock samples for a staged write — `(snapshot, mid)`.
    /// `(None, None)` if no write activity.
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
/// Enabled only when the debugger needs watchpoint matching — the
/// non-recording path stays allocation-free.
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
