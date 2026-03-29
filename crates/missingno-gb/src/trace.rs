use std::collections::BTreeMap;
use std::path::Path;

pub use gbtrace::{Trigger, BootRom, Profile};
use gbtrace::format::write::GbtraceWriter;
use gbtrace::format::read::derive_groups_pub;
use gbtrace::header::TraceHeader;
use gbtrace::profile::{field_type, field_nullable, FieldType};
use sha2::{Digest, Sha256};

use crate::GameBoy;
use crate::ppu::PpuTraceSnapshot;

/// Captures gbtrace-format execution traces from a GameBoy.
pub struct Tracer {
    writer: GbtraceWriter,
    fields: Vec<String>,
    field_cols: Vec<usize>,
    memory: BTreeMap<String, u16>,
    dot_count: u64,
    trigger: Trigger,
    needs_ppu_snapshot: bool,
    pix_buffer: String,
    vram_write_addr: u16,
    vram_write_data: u8,
}

const PPU_INTERNAL_FIELDS: &[&str] = &[
    "oam0_x", "oam0_id", "oam0_attr", "oam1_x", "oam1_id", "oam1_attr",
    "oam2_x", "oam2_id", "oam2_attr", "oam3_x", "oam3_id", "oam3_attr",
    "oam4_x", "oam4_id", "oam4_attr", "oam5_x", "oam5_id", "oam5_attr",
    "oam6_x", "oam6_id", "oam6_attr", "oam7_x", "oam7_id", "oam7_attr",
    "oam8_x", "oam8_id", "oam8_attr", "oam9_x", "oam9_id", "oam9_attr",
    "bgw_fifo_a", "bgw_fifo_b", "spr_fifo_a", "spr_fifo_b",
    "mask_pipe", "pal_pipe",
    "tfetch_state", "sfetch_state", "tile_temp_a", "tile_temp_b",
    "pix_count", "sprite_count", "scan_count",
    "rendering", "win_mode",
];

impl Tracer {
    pub fn create(
        path: impl AsRef<Path>,
        profile: &Profile,
        gb: &GameBoy,
        boot_rom: BootRom,
    ) -> Result<Self, gbtrace::Error> {
        let rom_sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(gb.cartridge().rom());
            format!("{:x}", hasher.finalize())
        };

        let trigger = profile.trigger.clone();

        let header = TraceHeader {
            _header: true,
            format_version: "0.1.0".into(),
            emulator: "missingno".into(),
            emulator_version: env!("CARGO_PKG_VERSION").into(),
            rom_sha256,
            model: "DMG-B".into(),
            boot_rom,
            profile: profile.name.clone(),
            fields: profile.fields.clone(),
            trigger: profile.trigger.clone(),
            notes: String::new(),
        };

        let groups = derive_groups_pub(&header.fields);
        let writer = GbtraceWriter::create(path, &header, &groups)?;

        // Build column index for each field
        let field_cols: Vec<usize> = (0..profile.fields.len()).collect();

        let needs_ppu_snapshot = profile
            .fields
            .iter()
            .any(|f| PPU_INTERNAL_FIELDS.contains(&f.as_str()));

        Ok(Self {
            writer,
            fields: profile.fields.clone(),
            field_cols,
            memory: profile.memory.clone(),
            dot_count: 0,
            trigger,
            needs_ppu_snapshot,
            pix_buffer: String::new(),
            vram_write_addr: 0,
            vram_write_data: 0,
        })
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger.clone()
    }

    pub fn push_pixel(&mut self, shade: u8) {
        self.pix_buffer.push((b'0' + (shade & 3)) as char);
    }

    pub fn push_vram_write(&mut self, addr: u16, data: u8) {
        self.vram_write_addr = addr;
        self.vram_write_data = data;
    }

    pub fn capture(&mut self, gb: &GameBoy) -> Result<(), gbtrace::Error> {
        let ppu_snap = if self.needs_ppu_snapshot {
            gb.ppu().trace_snapshot()
        } else {
            None
        };

        for (i, field) in self.fields.clone().iter().enumerate() {
            let col = self.field_cols[i];

            // Memory-mapped field
            if let Some(&addr) = self.memory.get(field.as_str()) {
                self.writer.set_u8(col, gb.peek(addr));
                continue;
            }

            match field.as_str() {
                // CPU
                "a" => self.writer.set_u8(col, gb.cpu().a),
                "f" => self.writer.set_u8(col, gb.cpu().flags.bits()),
                "b" => self.writer.set_u8(col, gb.cpu().b),
                "c" => self.writer.set_u8(col, gb.cpu().c),
                "d" => self.writer.set_u8(col, gb.cpu().d),
                "e" => self.writer.set_u8(col, gb.cpu().e),
                "h" => self.writer.set_u8(col, gb.cpu().h),
                "l" => self.writer.set_u8(col, gb.cpu().l),
                "sp" => self.writer.set_u16(col, gb.cpu().stack_pointer),
                "pc" => self.writer.set_u16(col, gb.cpu().instruction_pc),
                "ime" => self.writer.set_bool(col, gb.cpu().interrupts_enabled()),
                // PPU registers
                "lcdc" => self.writer.set_u8(col, gb.peek(0xFF40)),
                "stat" => self.writer.set_u8(col, gb.peek(0xFF41)),
                "ly" => self.writer.set_u8(col, gb.peek(0xFF44)),
                "lyc" => self.writer.set_u8(col, gb.peek(0xFF45)),
                "scy" => self.writer.set_u8(col, gb.peek(0xFF42)),
                "scx" => self.writer.set_u8(col, gb.peek(0xFF43)),
                "wy" => self.writer.set_u8(col, gb.peek(0xFF4A)),
                "wx" => self.writer.set_u8(col, gb.peek(0xFF4B)),
                "bgp" => self.writer.set_u8(col, gb.peek(0xFF47)),
                "obp0" => self.writer.set_u8(col, gb.peek(0xFF48)),
                "obp1" => self.writer.set_u8(col, gb.peek(0xFF49)),
                "dma" => self.writer.set_u8(col, gb.peek(0xFF46)),
                // Timer
                "div" => self.writer.set_u8(col, gb.peek(0xFF04)),
                "tima" => self.writer.set_u8(col, gb.peek(0xFF05)),
                "tma" => self.writer.set_u8(col, gb.peek(0xFF06)),
                "tac" => self.writer.set_u8(col, gb.peek(0xFF07)),
                // Interrupt
                "if_" => self.writer.set_u8(col, gb.peek(0xFF0F)),
                "ie" => self.writer.set_u8(col, gb.peek(0xFFFF)),
                // Serial
                "sb" => self.writer.set_u8(col, gb.peek(0xFF01)),
                "sc" => self.writer.set_u8(col, gb.peek(0xFF02)),
                // Pixel output
                "pix" => {
                    if self.pix_buffer.is_empty() {
                        self.writer.set_null(col);
                    } else {
                        self.writer.set_str(col, &self.pix_buffer);
                    }
                }
                "pix_x" => {
                    if let Some(snap) = &ppu_snap {
                        self.writer.set_u8(col, snap.pix_count);
                    } else {
                        self.writer.set_u8(col, 0);
                    }
                }
                // VRAM write tracking
                "vram_addr" => {
                    if self.vram_write_addr != 0 {
                        self.writer.set_u16(col, self.vram_write_addr);
                    } else {
                        self.writer.set_null(col);
                    }
                }
                "vram_data" => {
                    if self.vram_write_addr != 0 {
                        self.writer.set_u8(col, self.vram_write_data);
                    } else {
                        self.writer.set_null(col);
                    }
                }
                // PPU internal fields
                field_name if PPU_INTERNAL_FIELDS.contains(&field_name) => {
                    self.emit_ppu_field(col, field_name, &ppu_snap);
                }
                _ => {
                    // Unknown field — write a zero/null default
                    let ft = field_type(field);
                    if field_nullable(field) {
                        self.writer.set_null(col);
                    } else {
                        match ft {
                            FieldType::Bool => self.writer.set_bool(col, false),
                            FieldType::Str => self.writer.set_str(col, ""),
                            _ => self.writer.set_u8(col, 0),
                        }
                    }
                }
            }
        }

        self.pix_buffer.clear();
        self.vram_write_addr = 0;
        self.vram_write_data = 0;

        self.writer.finish_entry()
    }

    fn emit_ppu_field(&mut self, col: usize, field: &str, snap: &Option<PpuTraceSnapshot>) {
        let snap = match snap {
            Some(s) => s,
            None => {
                // No PPU snapshot — write type-appropriate zero
                match field {
                    "rendering" | "win_mode" => self.writer.set_bool(col, false),
                    _ => self.writer.set_u8(col, 0),
                }
                return;
            }
        };

        if let Some(rest) = field.strip_prefix("oam") {
            if let Some((idx_str, suffix)) = rest.split_once('_') {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx < 10 {
                        let val = match suffix {
                            "x" => snap.sprite_x[idx],
                            "id" => snap.sprite_id[idx],
                            "attr" => snap.sprite_attr[idx],
                            _ => 0,
                        };
                        self.writer.set_u8(col, val);
                        return;
                    }
                }
            }
        }

        match field {
            "bgw_fifo_a" => self.writer.set_u8(col, snap.bgw_fifo_a),
            "bgw_fifo_b" => self.writer.set_u8(col, snap.bgw_fifo_b),
            "spr_fifo_a" => self.writer.set_u8(col, snap.spr_fifo_a),
            "spr_fifo_b" => self.writer.set_u8(col, snap.spr_fifo_b),
            "mask_pipe" => self.writer.set_u8(col, snap.mask_pipe),
            "pal_pipe" => self.writer.set_u8(col, snap.pal_pipe),
            "tfetch_state" => self.writer.set_u8(col, snap.tfetch_state),
            "sfetch_state" => self.writer.set_u8(col, snap.sfetch_state),
            "tile_temp_a" => self.writer.set_u8(col, snap.tile_temp_a),
            "tile_temp_b" => self.writer.set_u8(col, snap.tile_temp_b),
            "pix_count" => self.writer.set_u8(col, snap.pix_count),
            "sprite_count" => self.writer.set_u8(col, snap.sprite_count),
            "scan_count" => self.writer.set_u8(col, snap.scan_count),
            "rendering" => self.writer.set_bool(col, snap.rendering),
            "win_mode" => self.writer.set_bool(col, snap.win_mode),
            _ => self.writer.set_u8(col, 0),
        }
    }

    pub fn mark_frame(&mut self) -> Result<(), gbtrace::Error> {
        self.writer.mark_frame(None)
    }

    pub fn advance_dot(&mut self) {
        self.dot_count += 1;
    }

    pub fn advance(&mut self, dots: u32) {
        self.dot_count += dots as u64;
    }

    pub fn dot_count(&self) -> u64 {
        self.dot_count
    }

    pub fn finish(self) -> Result<(), gbtrace::Error> {
        self.writer.finish()
    }
}
