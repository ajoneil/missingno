use std::collections::BTreeMap;
use std::path::Path;

pub use gbtrace::Trigger;
use gbtrace::{BootRom, ParquetTraceWriter, Profile, TraceEntry, TraceHeader};
use sha2::{Digest, Sha256};

use crate::GameBoy;
use crate::ppu::PpuTraceSnapshot;

/// Captures gbtrace-format execution traces from a GameBoy.
pub struct Tracer {
    writer: ParquetTraceWriter,
    fields: Vec<String>,
    memory: BTreeMap<String, u16>,
    dot_count: u64,
    trigger: Trigger,
    /// Whether any field requires a PPU trace snapshot.
    needs_ppu_snapshot: bool,
    /// Accumulated pixel output since the last capture, for the `pix` field.
    /// Shade characters ('0'-'3') appended by `push_pixel()`, drained on capture.
    pix_buffer: String,
}

/// PPU internal field names that require a PpuTraceSnapshot.
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
    "rendering", "win_mode", "frame_num",
];

impl Tracer {
    /// Create a new tracer that writes to the given path.
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
            cy_unit: gbtrace::CycleUnit::Tcycle,
            notes: String::new(),
        };

        let writer = ParquetTraceWriter::create(path, &header)?;

        let needs_ppu_snapshot = profile
            .fields
            .iter()
            .any(|f| PPU_INTERNAL_FIELDS.contains(&f.as_str()));

        Ok(Self {
            writer,
            fields: profile.fields.clone(),
            memory: profile.memory.clone(),
            dot_count: 0,
            trigger,
            needs_ppu_snapshot,
            pix_buffer: String::new(),
        })
    }

    /// The trigger granularity for this trace.
    pub fn trigger(&self) -> Trigger {
        self.trigger.clone()
    }

    /// Record a pixel output from the PPU. Call this for each pixel
    /// returned by `PhaseResult` between captures.
    pub fn push_pixel(&mut self, shade: u8) {
        self.pix_buffer.push((b'0' + (shade & 3)) as char);
    }

    /// Capture the current state and write a trace entry.
    pub fn capture(&mut self, gb: &GameBoy) -> Result<(), gbtrace::Error> {
        let mut entry = TraceEntry::new();

        // Take PPU snapshot once if any field needs it.
        let ppu_snap = if self.needs_ppu_snapshot {
            gb.ppu().trace_snapshot()
        } else {
            None
        };

        for field in &self.fields {
            // Check if this is a memory-mapped field first.
            if let Some(&addr) = self.memory.get(field) {
                entry.set_u8(field, gb.peek(addr));
                continue;
            }

            match field.as_str() {
                "cy" => entry.set_cy(self.dot_count),
                // CPU
                "a" => entry.set_u8("a", gb.cpu().a),
                "f" => entry.set_u8("f", gb.cpu().flags.bits()),
                "b" => entry.set_u8("b", gb.cpu().b),
                "c" => entry.set_u8("c", gb.cpu().c),
                "d" => entry.set_u8("d", gb.cpu().d),
                "e" => entry.set_u8("e", gb.cpu().e),
                "h" => entry.set_u8("h", gb.cpu().h),
                "l" => entry.set_u8("l", gb.cpu().l),
                "sp" => entry.set_u16("sp", gb.cpu().stack_pointer),
                "pc" => entry.set_u16("pc", gb.cpu().program_counter),
                "op" => entry.set_u8("op", gb.peek(gb.cpu().program_counter)),
                "ime" => entry.set_bool("ime", gb.cpu().interrupts_enabled()),
                // PPU registers
                "lcdc" => entry.set_u8("lcdc", gb.peek(0xFF40)),
                "stat" => entry.set_u8("stat", gb.peek(0xFF41)),
                "ly" => entry.set_u8("ly", gb.peek(0xFF44)),
                "lyc" => entry.set_u8("lyc", gb.peek(0xFF45)),
                "scy" => entry.set_u8("scy", gb.peek(0xFF42)),
                "scx" => entry.set_u8("scx", gb.peek(0xFF43)),
                "wy" => entry.set_u8("wy", gb.peek(0xFF4A)),
                "wx" => entry.set_u8("wx", gb.peek(0xFF4B)),
                "bgp" => entry.set_u8("bgp", gb.peek(0xFF47)),
                "obp0" => entry.set_u8("obp0", gb.peek(0xFF48)),
                "obp1" => entry.set_u8("obp1", gb.peek(0xFF49)),
                "dma" => entry.set_u8("dma", gb.peek(0xFF46)),
                // Timer
                "div" => entry.set_u8("div", gb.peek(0xFF04)),
                "tima" => entry.set_u8("tima", gb.peek(0xFF05)),
                "tma" => entry.set_u8("tma", gb.peek(0xFF06)),
                "tac" => entry.set_u8("tac", gb.peek(0xFF07)),
                // Interrupt
                "if_" => entry.set_u8("if_", gb.peek(0xFF0F)),
                "ie" => entry.set_u8("ie", gb.peek(0xFFFF)),
                // Serial
                "sb" => entry.set_u8("sb", gb.peek(0xFF01)),
                "sc" => entry.set_u8("sc", gb.peek(0xFF02)),
                // Pixel output
                "pix" => {
                    entry.set_str("pix", &self.pix_buffer);
                }
                "pix_x" => {
                    if let Some(snap) = &ppu_snap {
                        entry.set_u8("pix_x", snap.pix_count);
                    }
                }
                // PPU internal fields — use snapshot
                field_name if PPU_INTERNAL_FIELDS.contains(&field_name) => {
                    Self::emit_ppu_field(&mut entry, field_name, &ppu_snap);
                }
                _ => {}
            }
        }

        // Drain the pixel buffer after emitting — it covers the interval
        // since the last capture.
        self.pix_buffer.clear();

        self.writer.write_entry(&entry)
    }

    /// Write a single PPU internal field into a trace entry.
    fn emit_ppu_field(entry: &mut TraceEntry, field: &str, snap: &Option<PpuTraceSnapshot>) {
        let snap = match snap {
            Some(s) => s,
            None => return, // LCD off — no pipeline state
        };

        // Sprite store: oamN_x, oamN_id, oamN_attr
        if let Some(rest) = field.strip_prefix("oam") {
            if let Some((idx_str, suffix)) = rest.split_once('_') {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx < 10 {
                        let val = match suffix {
                            "x" => snap.sprite_x[idx],
                            "id" => snap.sprite_id[idx],
                            "attr" => snap.sprite_attr[idx],
                            _ => return,
                        };
                        entry.set_u8(field, val);
                        return;
                    }
                }
            }
        }

        match field {
            // Pixel FIFO
            "bgw_fifo_a" => entry.set_u8(field, snap.bgw_fifo_a),
            "bgw_fifo_b" => entry.set_u8(field, snap.bgw_fifo_b),
            "spr_fifo_a" => entry.set_u8(field, snap.spr_fifo_a),
            "spr_fifo_b" => entry.set_u8(field, snap.spr_fifo_b),
            "mask_pipe" => entry.set_u8(field, snap.mask_pipe),
            "pal_pipe" => entry.set_u8(field, snap.pal_pipe),
            // Fetcher
            "tfetch_state" => entry.set_u8(field, snap.tfetch_state),
            "sfetch_state" => entry.set_u8(field, snap.sfetch_state),
            "tile_temp_a" => entry.set_u8(field, snap.tile_temp_a),
            "tile_temp_b" => entry.set_u8(field, snap.tile_temp_b),
            // Counters
            "pix_count" => entry.set_u8(field, snap.pix_count),
            "sprite_count" => entry.set_u8(field, snap.sprite_count),
            "scan_count" => entry.set_u8(field, snap.scan_count),
            // Flags
            "rendering" => entry.set_bool(field, snap.rendering),
            "win_mode" => entry.set_bool(field, snap.win_mode),
            // Frame tracking
            "frame_num" => entry.set_u16(field, snap.frame_num),
            _ => {}
        }
    }

    /// Mark a frame boundary at the current position. Call at VBlank
    /// so the viewer can split frames correctly.
    pub fn mark_frame(&mut self) -> Result<(), gbtrace::Error> {
        self.writer.mark_frame()
    }

    /// Advance the dot counter by one T-cycle.
    pub fn advance_dot(&mut self) {
        self.dot_count += 1;
    }

    /// Advance the dot counter by multiple T-cycles (for instruction-level tracing).
    pub fn advance(&mut self, dots: u32) {
        self.dot_count += dots as u64;
    }

    /// Current T-cycle count.
    pub fn dot_count(&self) -> u64 {
        self.dot_count
    }

    /// Flush and finalize the trace file.
    pub fn finish(self) -> Result<(), gbtrace::Error> {
        self.writer.finish()
    }
}
