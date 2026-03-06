// --- FramePhase: top-level PPU rendering lifecycle ---

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    memory::{Oam, Vram},
};

use super::{Mode, PipelineSnapshot, RenderPhase, Rendering};

pub enum FramePhase {
    ActiveDisplay(Rendering),
    VerticalBlank,
}

impl FramePhase {
    pub fn new() -> Self {
        Self::ActiveDisplay(Rendering::new())
    }

    /// Create a PPU for an LCD-on transition (LCDC bit 7 set after being
    /// clear). The first line reports mode 0 in STAT until the OAM scan
    /// begins internally.
    pub fn new_lcd_on() -> Self {
        Self::ActiveDisplay(Rendering::new_lcd_on())
    }

    pub fn mode(&self, video: &VideoControl) -> Mode {
        match self {
            FramePhase::ActiveDisplay(rendering) => rendering.mode(video),
            FramePhase::VerticalBlank => Mode::VerticalBlank,
        }
    }

    pub fn stat_mode(&self, video: &VideoControl) -> Mode {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => {
                Mode::HorizontalBlank
            }
            FramePhase::ActiveDisplay(rendering) => rendering.stat_mode(video),
            FramePhase::VerticalBlank => Mode::VerticalBlank,
        }
    }

    pub fn interrupt_mode(&self, video: &VideoControl) -> Mode {
        match self {
            // During LCD startup, no STAT condition signal is active on hardware
            // (TARU/TAPA/PARU/ROPO are all low). Mode::Drawing has no matching
            // STAT enable bit, so returning it suppresses all mode-based STAT
            // conditions without needing a special enum variant.
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => {
                Mode::Drawing
            }
            // On hardware, Mode 1 STAT fires at clock 4 of line 144, not clock 0.
            // The internal mode-for-interrupt doesn't transition to Mode 1 until
            // 4 dots after VBlank entry.
            FramePhase::VerticalBlank if video.ly() == 144 && video.dot() < 4 => {
                Mode::HorizontalBlank
            }
            _ => self.mode(video),
        }
    }

    pub fn mode2_interrupt_active(&self, video: &VideoControl) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => false,
            FramePhase::ActiveDisplay(rendering) => rendering.mode2_interrupt_active(video),
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn oam_locked(&self) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => false,
            FramePhase::ActiveDisplay(rendering) => rendering.oam_locked(),
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn vram_locked(&self) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => false,
            FramePhase::ActiveDisplay(rendering) => rendering.vram_locked(),
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn oam_write_locked(&self) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => false,
            FramePhase::ActiveDisplay(rendering) => rendering.oam_write_locked(),
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn vram_write_locked(&self) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) if rendering.lcd_turning_on => false,
            FramePhase::ActiveDisplay(rendering) => rendering.vram_write_locked(),
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn is_rendering(&self) -> bool {
        match self {
            FramePhase::ActiveDisplay(rendering) => {
                matches!(
                    rendering.render_phase,
                    RenderPhase::Drawing | RenderPhase::DrawingComplete
                )
            }
            FramePhase::VerticalBlank => false,
        }
    }

    pub fn scanner_oam_address(&self) -> Option<u8> {
        match self {
            FramePhase::ActiveDisplay(rendering) => rendering.scanner_oam_address(),
            FramePhase::VerticalBlank => None,
        }
    }

    pub fn pipeline_state(&self) -> Option<PipelineSnapshot> {
        match self {
            FramePhase::ActiveDisplay(rendering) => Some(rendering.pipeline_state()),
            FramePhase::VerticalBlank => None,
        }
    }

    /// DELTA_EVEN half of a dot tick: fetcher control, mode transitions.
    pub fn tcycle_even(&mut self, regs: &PipelineRegisters, video: &VideoControl, vram: &Vram) {
        match self {
            FramePhase::ActiveDisplay(rendering) => {
                rendering.half_even(regs, video, vram);
            }
            FramePhase::VerticalBlank => {}
        }
    }

    /// DELTA_ODD half of a dot tick: pixel output phase.
    pub fn tcycle_odd(
        &mut self,
        regs: &PipelineRegisters,
        video: &VideoControl,
        oam: &Oam,
        vram: &Vram,
    ) {
        match self {
            FramePhase::ActiveDisplay(rendering) => {
                rendering.half_odd(regs, video, oam, vram);
            }
            FramePhase::VerticalBlank => {}
        }
    }
}
