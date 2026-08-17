/// Double-buffered LCD framebuffer, abstracted over its pixel storage so
/// the shared core can drive a DMG shade buffer or a CGB color buffer.
pub trait ScreenBuffer: Default + Clone {
    type Pixel: Copy;
    fn draw_pixel(&mut self, x: u8, y: u8, pixel: Self::Pixel);
    /// Swap back→front and clear back. Returns true for `new_screen` tracking.
    fn present(&mut self) -> bool;
    fn blank(&mut self);
    /// The displayed buffer as flat greyscale bytes, one per pixel on the DMG
    /// reference shade ramp — the currency the shade-pattern references and the
    /// hex-glyph readers share across consoles.
    fn to_greyscale_bytes(&self) -> Vec<u8>;
    /// Seed the displayed (front) buffer from a save state's framebuffer bytes,
    /// so the first frame after a restore matches the save. Each console decodes
    /// its own pixel format (DMG shade indices, CGB little-endian RGB555).
    fn restore(&mut self, bytes: &[u8]);
}

/// CGB-only console-level arbitration state, relocated off the shared
/// [`Console`](crate::Console) so a DMG build carries none of it. The CGB model
/// owns the real storage; the DMG model is a ZST `()`, since none of these
/// paths — the speed-switch blackout, the HDMA bus-park, the VRAM-source
/// OAM-zero conflict — exist on the DMG.
pub trait ConsoleShadow {
    /// The master-edge count a double-speed switch blackout began on; the
    /// elapsed held edges are `master_edge - anchor`. Re-anchored at each switch.
    fn blackout_anchor(&self) -> u64;
    fn set_blackout_anchor(&mut self, edge: u64);

    /// A VRAM DMA is holding the CPU clock this M-cycle (bus master owns the bus).
    fn dma_cpu_hold(&self) -> bool;
    fn set_dma_cpu_hold(&mut self, held: bool);

    /// A bus master owns the VRAM/external bus this M-cycle, so a CPU access
    /// starting here waits for release (per-bus wait states, the sibling of the
    /// whole-bandwidth `dma_cpu_hold`). Computed at each M-boundary.
    fn bus_suspended(&self) -> bool;
    fn set_bus_suspended(&mut self, suspended: bool);

    /// The VRAM-DMA trigger's bus claim committed this M-cycle (consumed at the
    /// next M-cycle pick and then cleared).
    fn vram_dma_claim(&self) -> VramDmaClaim;
    fn set_vram_dma_claim(&mut self, claim: VramDmaClaim);
    fn clear_vram_dma_claim(&mut self);

    /// OAM offset whose DMA-deposited byte a VRAM-source bus conflict forces to
    /// `$00`, drained at the M-cycle-boundary fall.
    fn set_dma_conflict_oam_zero(&mut self, offset: Option<u8>);
    fn take_dma_conflict_oam_zero(&mut self) -> Option<u8>;
}

impl ConsoleShadow for () {
    fn blackout_anchor(&self) -> u64 {
        0
    }
    fn set_blackout_anchor(&mut self, _edge: u64) {}
    fn dma_cpu_hold(&self) -> bool {
        false
    }
    fn set_dma_cpu_hold(&mut self, _held: bool) {}
    fn bus_suspended(&self) -> bool {
        false
    }
    fn set_bus_suspended(&mut self, _suspended: bool) {}
    fn vram_dma_claim(&self) -> VramDmaClaim {
        VramDmaClaim::default()
    }
    fn set_vram_dma_claim(&mut self, _claim: VramDmaClaim) {}
    fn clear_vram_dma_claim(&mut self) {}
    fn set_dma_conflict_oam_zero(&mut self, _offset: Option<u8>) {}
    fn take_dma_conflict_oam_zero(&mut self) -> Option<u8> {
        None
    }
}

/// The HDMA trigger's bus claim committed on a fall: `standing` marks a
/// claim that aged through its synchronizer stage before committing (it
/// wins the bus race against the halt-release fetch).
#[derive(Copy, Clone, Default)]
pub struct VramDmaClaim {
    pub committed: bool,
    pub standing: bool,
}
