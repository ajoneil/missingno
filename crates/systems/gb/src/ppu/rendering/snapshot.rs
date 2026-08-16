//! The debugger and trace faces of the pixel pipeline: plain snapshots of
//! state that is otherwise private to [`Rendering`].

use super::Rendering;
use crate::ppu::{
    BgFifoCell, ObjFifoCell, PipelineRegisters, PpuModel, VideoControl, memory::Oam,
    types::sprites::SpriteId,
};

pub use crate::ppu::draw::sprite_fetch::SpriteFetchPhase;
use crate::ppu::draw::sprite_fetch::SpriteState;

pub struct SpriteStoreSnapshot {
    pub count: u8,
    pub fetched: u16,
    pub entries: Vec<SpriteStoreEntrySnapshot>,
}
pub struct SpriteStoreEntrySnapshot {
    pub oam_index: u8,
    pub line_offset: u8,
    pub x: u8,
    pub fetched: bool,
}
/// morepork `ppu_internal` snapshot. Field names match the morepork spec.
pub struct PpuTraceSnapshot {
    pub sprite_x: [u8; 10],
    pub sprite_id: [u8; 10],
    pub sprite_attr: [u8; 10],
    pub bgw_fifo_a: u8,
    pub bgw_fifo_b: u8,
    pub spr_fifo_a: u8,
    pub spr_fifo_b: u8,
    pub pal_pipe: u8,
    pub tfetch_state: u8,
    /// 0 when no sprite fetch active.
    pub sfetch_state: u8,
    pub tile_temp_a: u8,
    pub tile_temp_b: u8,
    pub pix_count: u8,
    pub sprite_count: u8,
    pub scan_count: u8,
    pub rendering: bool,
    pub win_mode: bool,
    pub frame_num: u16,
}
/// Debugger pipeline snapshot, serialised by the headless debugger JSON API.
pub struct PipelineSnapshot {
    pub pixel_counter: u8,
    /// Mode 3 active (XYMU; inverted polarity).
    pub rendering_active: bool,
    pub bg_low: u8,
    pub bg_high: u8,
    pub obj_low: u8,
    pub obj_high: u8,
    pub obj_palette: u8,
    pub obj_priority: u8,
    pub sprite_fetch_phase: Option<SpriteFetchPhase>,
    pub sprite_tile_data: Option<(u8, u8)>,
    pub lcd_x: u8,
    pub fetch_counter: u8,
    /// Window-hit latch (RYDY).
    pub window_hit: bool,
    /// LCD pixel-emit gate (WUSA).
    pub pixel_gate: bool,
    /// Fine-scroll match for the cp_pad strobe (POVA).
    pub fine_scroll_match: bool,
    /// Fetcher-idle stage 3 (PYGO).
    pub fetcher_idle_stage_3: bool,
    /// Fetcher-ready output (POKY).
    pub fetcher_ready: bool,
    pub wx_triggered: bool,
    /// Video clock divider (WUVU).
    pub video_clock: bool,
    /// Scan-done flag (BYBA, dffr, XUPY-clocked).
    pub scan_done: bool,
    /// Prior-scan-done (DOBA, dffr, ALET-clocked).
    pub scan_done_prev: bool,
}

impl<P: PpuModel> Rendering<P> {
    pub(in crate::ppu) fn sprite_store_snapshot(&self) -> SpriteStoreSnapshot {
        let sprites = &self.scan.sprites_ref();
        SpriteStoreSnapshot {
            count: sprites.count,
            fetched: sprites.fetched,
            entries: (0..sprites.count as usize)
                .map(|i| {
                    let e = &sprites.entries[i];
                    SpriteStoreEntrySnapshot {
                        oam_index: e.oam_index,
                        line_offset: e.line_offset,
                        x: e.x,
                        fetched: sprites.fetched & (1 << i) != 0,
                    }
                })
                .collect(),
        }
    }
    pub(in crate::ppu) fn trace_snapshot(&self, oam: &Oam) -> PpuTraceSnapshot {
        let sprites = self.scan.sprites_ref();
        let mut sprite_x = [0u8; 10];
        let mut sprite_id = [0u8; 10];
        let mut sprite_attr = [0u8; 10];
        for i in 0..sprites.count as usize {
            let entry = &sprites.entries[i];
            sprite_x[i] = entry.x;
            let oam_sprite = oam.sprite(SpriteId(entry.oam_index));
            sprite_id[i] = oam_sprite.tile.0;
            sprite_attr[i] = oam_sprite.attributes.0;
        }

        let (bg_low, bg_high) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, _obj_priority) = P::obj_trace(&self.obj_fifo);

        let sfetch_state = match &self.sprite_state {
            SpriteState::Fetching(sf) => sf.fetch_counter(),
            SpriteState::Idle => 0,
        };

        PpuTraceSnapshot {
            sprite_x,
            sprite_id,
            sprite_attr,
            bgw_fifo_a: bg_low,
            bgw_fifo_b: bg_high,
            spr_fifo_a: obj_low,
            spr_fifo_b: obj_high,
            pal_pipe: obj_palette,
            tfetch_state: self.fetcher.fetch_counter,
            sfetch_state,
            tile_temp_a: self.fetcher.tile_data_low(),
            tile_temp_b: self.fetcher.tile_data_high(),
            pix_count: self.pixel_counter.value(),
            sprite_count: sprites.count,
            scan_count: self.scan.scan_counter_entry(),
            rendering: self.hblank.rendering_active(),
            win_mode: self.window.window_rendered(),
            frame_num: 0,
        }
    }
    /// The background shifter's 8 stages, MSB-first (cell 0 = the next pixel to
    /// pop): the 2-bit colour and the tile's BG palette (CGB) riding all 8.
    pub(in crate::ppu) fn bg_fifo_cells(&self) -> [BgFifoCell; 8] {
        let (low, high) = self.bg_shifter.registers();
        let palette = P::bg_cell_palette(self.bg_shifter.cell());
        std::array::from_fn(|i| {
            let bit = 7 - i as u8;
            BgFifoCell {
                color: (((high >> bit) & 1) << 1) | ((low >> bit) & 1),
                palette,
            }
        })
    }
    /// The object FIFO decoded into its 8 stages, MSB-first.
    pub(in crate::ppu) fn obj_fifo_cells(&self) -> [ObjFifoCell; 8] {
        P::obj_fifo_cells(&self.obj_fifo)
    }
    pub fn pipeline_state(
        &self,
        video: &VideoControl,
        regs: &PipelineRegisters,
    ) -> PipelineSnapshot {
        let (bg_low, bg_high) = self.bg_shifter.registers();
        let (obj_low, obj_high, obj_palette, obj_priority) = P::obj_trace(&self.obj_fifo);
        let (sprite_fetch_phase, sprite_tile_data) = match &self.sprite_state {
            SpriteState::Fetching(sf) => {
                (Some(SpriteFetchPhase::FetchingData), Some(sf.tile_data()))
            }
            SpriteState::Idle => (None, None),
        };
        PipelineSnapshot {
            pixel_counter: self.pixel_counter.value(),
            rendering_active: self.hblank.rendering_active(),
            bg_low,
            bg_high,
            obj_low,
            obj_high,
            obj_palette,
            obj_priority,
            sprite_fetch_phase,
            sprite_tile_data,
            lcd_x: self.lcd.lcd_x(),
            fetch_counter: self.fetcher.fetch_counter,
            window_hit: self.window.window_hit(),
            pixel_gate: self.lcd.pixel_gate(),
            fine_scroll_match: self.lcd.fine_scroll_match(),
            fetcher_idle_stage_3: self.cascade.fetch_complete_stage_3(),
            fetcher_ready: self.cascade.pixel_data_ready(),
            wx_triggered: self.window.wx_triggered(regs, Self::window_synced()),
            video_clock: video.scan_clock(),
            scan_done: self.scan.scan_done_flag(),
            scan_done_prev: self.scan.scan_done_prev(),
        }
    }
}
