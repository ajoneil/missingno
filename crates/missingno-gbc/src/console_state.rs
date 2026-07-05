use missingno_gb::{ConsoleShadow, VramDmaClaim};

/// The CGB console-level arbitration state, relocated off the shared
/// [`Console`] so a DMG build carries none of it (its `ConsoleState` is a ZST
/// `()`). Holds the speed-switch blackout anchor, the HDMA bus-park, and the
/// VRAM-source OAM-zero conflict store.
#[derive(Default)]
pub struct CgbConsoleState {
    blackout_anchor: u64,
    dma_cpu_hold: bool,
    bus_suspended: bool,
    vram_dma_claim: VramDmaClaim,
    dma_conflict_oam_zero: Option<u8>,
}

impl ConsoleShadow for CgbConsoleState {
    fn blackout_anchor(&self) -> u64 {
        self.blackout_anchor
    }
    fn set_blackout_anchor(&mut self, edge: u64) {
        self.blackout_anchor = edge;
    }
    fn dma_cpu_hold(&self) -> bool {
        self.dma_cpu_hold
    }
    fn set_dma_cpu_hold(&mut self, held: bool) {
        self.dma_cpu_hold = held;
    }
    fn bus_suspended(&self) -> bool {
        self.bus_suspended
    }
    fn set_bus_suspended(&mut self, suspended: bool) {
        self.bus_suspended = suspended;
    }
    fn vram_dma_claim(&self) -> VramDmaClaim {
        self.vram_dma_claim
    }
    fn set_vram_dma_claim(&mut self, claim: VramDmaClaim) {
        self.vram_dma_claim = claim;
    }
    fn clear_vram_dma_claim(&mut self) {
        self.vram_dma_claim = VramDmaClaim::default();
    }
    fn dma_conflict_oam_zero(&self) -> Option<u8> {
        self.dma_conflict_oam_zero
    }
    fn set_dma_conflict_oam_zero(&mut self, offset: Option<u8>) {
        self.dma_conflict_oam_zero = offset;
    }
    fn take_dma_conflict_oam_zero(&mut self) -> Option<u8> {
        self.dma_conflict_oam_zero.take()
    }
}
