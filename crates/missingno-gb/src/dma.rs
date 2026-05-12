use super::memory::Bus;

/// Which side is driving the OAM SRAM address bus this dot.
///
/// During DMA, all three PPU-side OAM bus drivers (CPU via ASAM, Parse
/// via APAR, Render via BETE) tri-state through the shared
/// `boge = NOT(dma_run)` upstream gate (spec §4.9.4). DMA owns the bus
/// uncontested while a transfer is active.
pub enum OamBusOwner {
    /// A mode-appropriate PPU-side driver owns the bus; the §5 Stage-1
    /// `oam_data_latch` enable is free to fire and capture the addressed
    /// byte-pair into the (Y, X) latches.
    Ppu,
    /// DMA drives the bus, asserting this OAM byte offset (0..=159). The
    /// §5 Stage-1 capture enable is gated off (mode2 = AND2(boge, BESU.q)
    /// = 0 → ajep = 1 → oam_data_latch = 0), so the (Y, X) latches HOLD
    /// their prior values throughout the overlap (spec §4.9.4.1).
    Dma(u8),
}

/// Pre-transfer delay state for OAM DMA. `None` means the DMA is
/// actively transferring bytes with bus conflicts enabled.
pub enum DmaDelay {
    /// Fresh DMA: M-cycles remaining before bus conflicts activate and
    /// the first byte transfers. During this delay OAM is still accessible.
    Startup(u8),
    /// Restarted DMA: M-cycles remaining before byte transfers begin.
    /// Bus conflicts are already active (inherited from the previous DMA).
    Transfer(u8),
}

/// State for an in-progress OAM DMA transfer.
pub struct DmaTransfer {
    /// Base source address (page * 0x100).
    pub source: u16,
    /// Which bus the DMA source resides on.
    source_bus: Bus,
    /// Next byte index to transfer (0..160).
    pub byte_index: u8,
    /// Pre-transfer delay countdown, or `None` if actively transferring.
    pub delay: Option<DmaDelay>,
}

/// OAM DMA controller state.
pub struct Dma {
    /// In-progress transfer, if any.
    pub transfer: Option<DmaTransfer>,
    /// Last value written to the DMA register (0xFF46).
    source_register: u8,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            transfer: None,
            source_register: 0xFF,
        }
    }

    /// The last value written to the DMA register (0xFF46).
    pub fn source_register(&self) -> u8 {
        self.source_register
    }

    /// If DMA is actively conflicting with a bus (past startup delay),
    /// returns which bus it occupies. Returns `None` when idle or still
    /// in startup.
    pub fn is_active_on_bus(&self) -> Option<Bus> {
        self.transfer.as_ref().and_then(|t| {
            if matches!(t.delay, Some(DmaDelay::Startup(_))) {
                None
            } else {
                Some(t.source_bus)
            }
        })
    }

    /// Which side is driving the OAM SRAM address bus this dot — DMA
    /// while a transfer is past its startup delay (`boge = NOT(dma_run)`
    /// tri-states the three PPU-side drivers, spec §4.9.4), otherwise
    /// the PPU.
    pub fn oam_bus_owner(&self) -> OamBusOwner {
        match self.transfer.as_ref() {
            Some(t) if !matches!(t.delay, Some(DmaDelay::Startup(_))) => {
                OamBusOwner::Dma(t.byte_index.min(159))
            }
            _ => OamBusOwner::Ppu,
        }
    }

    /// Advance DMA by one M-cycle. Returns the (source address,
    /// destination offset) pair when a byte should be transferred.
    pub fn mcycle(&mut self) -> Option<(u16, u8)> {
        let t = self.transfer.as_mut()?;
        match &mut t.delay {
            Some(DmaDelay::Startup(n) | DmaDelay::Transfer(n)) if *n > 1 => {
                *n -= 1;
                None
            }
            Some(_) => {
                t.delay = None;
                None
            }
            None => {
                let src_addr = t.source + t.byte_index as u16;
                let dst_offset = t.byte_index;
                t.byte_index += 1;
                if t.byte_index == 160 {
                    self.transfer = None;
                }
                Some((src_addr, dst_offset))
            }
        }
    }

    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::DmaSnapshot) -> Dma {
        if !snap.active {
            return Dma::new();
        }
        let delay = if snap.delay_remaining == 0 {
            None
        } else if snap.delay_remaining & 0x80 != 0 {
            Some(DmaDelay::Startup(snap.delay_remaining & 0x7F))
        } else {
            Some(DmaDelay::Transfer(snap.delay_remaining))
        };
        Dma {
            transfer: Some(DmaTransfer::new(snap.source, snap.byte_index, delay)),
            source_register: (snap.source >> 8) as u8,
        }
    }

    /// Start a new OAM DMA transfer. If a transfer is already active
    /// (past startup), bus conflicts remain in effect during the new
    /// startup period.
    pub fn begin_transfer(&mut self, source: u8) {
        let active = self
            .transfer
            .as_ref()
            .is_some_and(|t| !matches!(t.delay, Some(DmaDelay::Startup(_))));
        let source_addr = source as u16 * 0x100;
        self.source_register = source;
        self.transfer = Some(DmaTransfer {
            source: source_addr,
            source_bus: Bus::of(source_addr).unwrap_or(Bus::External),
            byte_index: 0,
            delay: Some(if active {
                DmaDelay::Transfer(2)
            } else {
                DmaDelay::Startup(2)
            }),
        });
    }
}

impl DmaTransfer {
    pub fn new(source: u16, byte_index: u8, delay: Option<DmaDelay>) -> Self {
        Self {
            source,
            source_bus: Bus::of(source).unwrap_or(Bus::External),
            byte_index,
            delay,
        }
    }
}
