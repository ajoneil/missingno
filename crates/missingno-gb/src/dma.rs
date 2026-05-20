use super::memory::Bus;

/// Which side drives the OAM SRAM address bus this T-cycle. During DMA
/// the CPU/Parse/Render OAM drivers all tri-state via `boge = !dma_run`,
/// so DMA owns the bus uncontested.
pub enum OamBusOwner {
    /// A PPU-side driver owns the bus; the OAM data latch captures the
    /// addressed byte-pair as normal.
    Ppu,
    /// DMA drives the bus at this OAM byte offset (0..=159). The OAM
    /// data latch is gated off (mode2 forced low via `boge`), so the
    /// (Y, X) latches hold their prior values throughout the overlap.
    Dma(u8),
}

/// Pre-transfer delay before bytes start moving.
pub enum DmaDelay {
    /// Fresh DMA: M-cycles remaining before bus conflicts activate and
    /// the first byte transfers. OAM is still accessible during this
    /// window.
    Startup(u8),
    /// Restarted DMA: M-cycles remaining before byte transfers begin.
    /// Bus conflicts are already active (inherited from the prior DMA).
    Transfer(u8),
}

/// State for an in-progress OAM DMA transfer.
pub struct DmaTransfer {
    /// Base source address (page * 0x100).
    pub(crate) source: u16,
    /// Which bus the DMA source resides on.
    source_bus: Bus,
    /// Next byte index to transfer (0..160).
    pub(crate) byte_index: u8,
    /// Pre-transfer delay countdown, or `None` once actively transferring.
    pub(crate) delay: Option<DmaDelay>,
}

/// OAM DMA controller.
pub struct Dma {
    pub(crate) transfer: Option<DmaTransfer>,
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

    /// Last value written to the DMA register (0xFF46).
    pub fn source_register(&self) -> u8 {
        self.source_register
    }

    /// Whether the DMA source actively drives the data bus through the
    /// OAM-write half-cycle. True for any source where the on-package
    /// WRAM die's chip-select asserts under DMA — its CS is `!cs_n_pad
    /// && a[14]=1`, with no `a[13]` qualifier and no `fexx_ffxx_n`
    /// qualifier (those exclusions live inside the CPU die's CS chain,
    /// which `tyho_inst` bypasses during DMA). Pages $C0..$FF all
    /// satisfy this, so the WRAM driver stays live through the OAM
    /// write phase and a same-bus CPU strobe open-drains as
    /// `src_byte AND cpu_byte`. False for cartridge ROM / SRAM / VRAM,
    /// which latch and release before the write phase, letting the
    /// CPU's value land cleanly.
    pub fn source_drives_write_phase(&self) -> bool {
        matches!(self.source_register, 0xC0..=0xFF)
    }

    /// Bus that DMA is actively conflicting with (past the startup
    /// delay). `None` when idle or still in startup.
    pub fn is_active_on_bus(&self) -> Option<Bus> {
        self.transfer.as_ref().and_then(|t| {
            if matches!(t.delay, Some(DmaDelay::Startup(_))) {
                None
            } else {
                Some(t.source_bus)
            }
        })
    }

    /// Which side drives the OAM SRAM address bus this T-cycle — DMA
    /// past its startup delay, otherwise the PPU.
    pub fn oam_bus_owner(&self) -> OamBusOwner {
        match self.transfer.as_ref() {
            Some(t) if !matches!(t.delay, Some(DmaDelay::Startup(_))) => {
                OamBusOwner::Dma(t.byte_index.min(159))
            }
            _ => OamBusOwner::Ppu,
        }
    }

    /// `(source address, destination offset)` of the byte the next
    /// `mcycle()` will transfer, without mutating state. None during
    /// startup, restart delay, or after the 160th byte.
    pub fn peek_transfer(&self) -> Option<(u16, u8)> {
        let t = self.transfer.as_ref()?;
        if t.delay.is_some() || t.byte_index >= 160 {
            return None;
        }
        Some((t.source + t.byte_index as u16, t.byte_index))
    }

    /// Advance DMA by one M-cycle. Returns the `(source address,
    /// destination offset)` pair when a byte should be transferred.
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

    /// Start a new OAM DMA transfer. If a transfer is already active
    /// past startup, bus conflicts remain in effect during the new
    /// startup period.
    pub fn begin_transfer(&mut self, source: u8) {
        let active = self
            .transfer
            .as_ref()
            .is_some_and(|t| !matches!(t.delay, Some(DmaDelay::Startup(_))));
        let source_addr = (source as u16) * 0x100;
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

    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::DmaSnapshot) -> Dma {
        if !snap.active {
            return Dma::new();
        }
        let delay = match snap.delay_remaining {
            0 => None,
            n if n & 0x80 != 0 => Some(DmaDelay::Startup(n & 0x7F)),
            n => Some(DmaDelay::Transfer(n)),
        };
        Dma {
            transfer: Some(DmaTransfer {
                source: snap.source,
                source_bus: Bus::of(snap.source).unwrap_or(Bus::External),
                byte_index: snap.byte_index,
                delay,
            }),
            source_register: (snap.source >> 8) as u8,
        }
    }
}
