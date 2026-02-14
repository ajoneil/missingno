use super::memory::Bus;

/// Pre-transfer delay state for OAM DMA. `None` means the DMA is
/// actively transferring bytes with bus conflicts enabled.
enum DmaDelay {
    /// Fresh DMA: M-cycles remaining before bus conflicts activate and
    /// the first byte transfers. During this delay OAM is still accessible.
    Startup(u8),
    /// Restarted DMA: M-cycles remaining before byte transfers begin.
    /// Bus conflicts are already active (inherited from the previous DMA).
    Transfer(u8),
}

/// State for an in-progress OAM DMA transfer.
struct DmaTransfer {
    /// Base source address (page * 0x100).
    source: u16,
    /// Which bus the DMA source resides on.
    source_bus: Bus,
    /// Next byte index to transfer (0..160).
    byte_index: u8,
    /// Pre-transfer delay countdown, or `None` if actively transferring.
    delay: Option<DmaDelay>,
}

/// OAM DMA controller state.
pub struct Dma {
    /// In-progress transfer, if any.
    transfer: Option<DmaTransfer>,
    /// Last value written to the DMA register (0xFF46).
    source_register: u8,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            transfer: None,
            source_register: 0,
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
