//! The SoC's shared CPU data bus (`cpu_port_d[7:0]`): its current
//! value, the M-cycle-boundary staging used to drive reads and writes
//! onto it at hardware-accurate dots, and the access-trace primitives
//! the debugger uses to observe traffic.

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
    /// CPU read M-cycle; latched by the CPU at end of M-cycle.
    pub data: u8,
}

impl CpuBus {
    pub(crate) fn new() -> Self {
        Self { data: 0xFF }
    }
}

/// A CPU bus write staged at the M-cycle boundary, applied at dot 2
/// of the write M-cycle. The CPU drives `cpu_port_d` at dot 2 (CUPA-
/// rising on the PPU side, `cpu_wr` asserted on the SM83 side); PPU
/// register latches are transparent during dots 2-3 and capture at
/// CUPA-falling (end of dot 3). Memory consumers commit at end-of-
/// M-cycle (fall of dot 3) via `write_byte`. The write VALUE lives
/// in `cpu_bus.data` — symmetric with `StagedBusRead` where the
/// value is also in `cpu_bus.data` (driven by the peripheral).
pub(crate) struct StagedBusWrite {
    pub(crate) address: u16,
    /// Whether the bus has been driven for this write.
    pub(crate) applied: bool,
    /// OAM/VRAM lock state at CUPA-rising (rise of dot 2 of the write
    /// M-cycle). Captured separately from the mid-CUPA and commit
    /// samples below — the three samples model the AJUJ-high window:
    /// a write lands if AJUJ is high at ANY edge during the CUPA
    /// strobe, so block iff locked at ALL three samples.
    /// None = non-OAM/VRAM address.
    pub(crate) locked_at_snapshot: Option<bool>,
    /// OAM/VRAM lock state at the mid-CUPA edge (fall of dot 2 of the
    /// write M-cycle). Catches the AJUJ-glitch window when AVAP fires
    /// this fall: BESU clears immediately (mode2↓), begin_rendering
    /// defers to the next rise (mode3↑), so this sample sees
    /// `mode2=0 AND mode3=0` → unlocked. The write then lands at the
    /// straddle even though both snap and commit see locked state.
    /// None = non-OAM/VRAM address.
    pub(crate) locked_at_mid: Option<bool>,
}

impl StagedBusWrite {
    pub(crate) fn new(address: u16) -> Self {
        Self {
            address,
            applied: false,
            locked_at_snapshot: None,
            locked_at_mid: None,
        }
    }
}

/// A CPU bus read staged at the M-cycle boundary, applied at dot 2
/// (matching `tobe`/`wafu` rising at hardware's dot 2.005). The
/// addressed peripheral's tri-state driver enables at this dot, the
/// bus settles to its source value, and the CPU latches at end of
/// M-cycle. Same-M-cycle peripheral state changes that fire after
/// dot 2 do not propagate to `cpu_port_d` in time for the latch.
pub(crate) struct StagedBusRead {
    pub(crate) address: u16,
    /// Whether the bus has been driven for this read.
    pub(crate) applied: bool,
}

impl StagedBusRead {
    pub(crate) fn new(address: u16) -> Self {
        Self {
            address,
            applied: false,
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
