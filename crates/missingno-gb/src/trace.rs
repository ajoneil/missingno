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

/// Pre-resolved emitter — determines how to read a field value at capture time.
enum Emitter {
    // CPU
    CpuA, CpuF, CpuB, CpuC, CpuD, CpuE, CpuH, CpuL,
    CpuSp, CpuPc, CpuIme,
    // IO read (PPU regs, timer, interrupt, serial, APU regs)
    IoRead(u16),
    // Memory read (profile [fields.memory])
    MemRead(u16),
    // PPU internal (needs snapshot)
    PpuInternal(PpuField),
    // APU internal
    Ch1Active, Ch1FreqCnt, Ch1EnvVol, Ch1Phase, Ch1SweepShadow, Ch1LenCnt,
    Ch2Active, Ch2FreqCnt, Ch2EnvVol, Ch2Phase, Ch2LenCnt,
    Ch3Active, Ch3FreqCnt, Ch3WaveIdx, Ch3Sample, Ch3LenCnt,
    Ch4Active, Ch4FreqCnt, Ch4EnvVol, Ch4Lfsr, Ch4LenCnt,
    // Pixel output
    Pix,
    PpuPixX,
    // VRAM write tracking
    VramAddr,
    VramData,
    // Wave RAM write tracking
    WaveAddr,
    WaveData,
    // Unknown — write type-appropriate zero/null
    Unknown(FieldType, bool),
}

enum PpuField {
    Oam { index: usize, component: OamComponent },
    BgwFifoA, BgwFifoB, SprFifoA, SprFifoB,
    MaskPipe, PalPipe,
    TfetchState, SfetchState, TileTempA, TileTempB,
    PixCount, SpriteCount, ScanCount,
    Rendering, WinMode,
}

#[derive(Copy, Clone)]
enum OamComponent { X, Id, Attr }

/// Resolved field: column index + how to emit.
struct ResolvedField {
    col: usize,
    emitter: Emitter,
}

/// Captures gbtrace-format execution traces from a GameBoy.
pub struct Tracer {
    writer: GbtraceWriter,
    emitters: Vec<ResolvedField>,
    dot_count: u64,
    trigger: Trigger,
    needs_ppu_snapshot: bool,
    pix_buffer: String,
    vram_write_addr: u16,
    vram_write_data: u8,
    wave_write_addr: u16,
    wave_write_data: u8,
}

static IO_FIELDS: &[(&str, u16)] = &[
    // PPU registers
    ("lcdc", 0xFF40), ("stat", 0xFF41), ("ly", 0xFF44), ("lyc", 0xFF45),
    ("scy", 0xFF42), ("scx", 0xFF43), ("wy", 0xFF4A), ("wx", 0xFF4B),
    ("bgp", 0xFF47), ("obp0", 0xFF48), ("obp1", 0xFF49), ("dma", 0xFF46),
    // Timer
    ("div", 0xFF04), ("tima", 0xFF05), ("tma", 0xFF06), ("tac", 0xFF07),
    // Interrupt
    ("if_", 0xFF0F), ("ie", 0xFFFF),
    // Serial
    ("sb", 0xFF01), ("sc", 0xFF02),
    // APU registers
    ("nr10", 0xFF10), ("nr11", 0xFF11), ("nr12", 0xFF12), ("nr13", 0xFF13), ("nr14", 0xFF14),
    ("nr21", 0xFF16), ("nr22", 0xFF17), ("nr23", 0xFF18), ("nr24", 0xFF19),
    ("nr30", 0xFF1A), ("nr31", 0xFF1B), ("nr32", 0xFF1C), ("nr33", 0xFF1D), ("nr34", 0xFF1E),
    ("nr41", 0xFF20), ("nr42", 0xFF21), ("nr43", 0xFF22), ("nr44", 0xFF23),
    ("nr50", 0xFF24), ("nr51", 0xFF25), ("nr52", 0xFF26),
];

fn resolve_emitter(field: &str, memory: &BTreeMap<String, u16>) -> Emitter {
    match field {
        // CPU
        "a" => Emitter::CpuA, "f" => Emitter::CpuF,
        "b" => Emitter::CpuB, "c" => Emitter::CpuC,
        "d" => Emitter::CpuD, "e" => Emitter::CpuE,
        "h" => Emitter::CpuH, "l" => Emitter::CpuL,
        "sp" => Emitter::CpuSp, "pc" => Emitter::CpuPc,
        "ime" => Emitter::CpuIme,
        // APU internal
        "ch1_active" => Emitter::Ch1Active,
        "ch1_freq_cnt" => Emitter::Ch1FreqCnt,
        "ch1_env_vol" => Emitter::Ch1EnvVol,
        "ch1_phase" => Emitter::Ch1Phase,
        "ch1_sweep_shadow" => Emitter::Ch1SweepShadow,
        "ch1_len_cnt" => Emitter::Ch1LenCnt,
        "ch2_active" => Emitter::Ch2Active,
        "ch2_freq_cnt" => Emitter::Ch2FreqCnt,
        "ch2_env_vol" => Emitter::Ch2EnvVol,
        "ch2_phase" => Emitter::Ch2Phase,
        "ch2_len_cnt" => Emitter::Ch2LenCnt,
        "ch3_active" => Emitter::Ch3Active,
        "ch3_freq_cnt" => Emitter::Ch3FreqCnt,
        "ch3_wave_idx" => Emitter::Ch3WaveIdx,
        "ch3_sample" => Emitter::Ch3Sample,
        "ch3_len_cnt" => Emitter::Ch3LenCnt,
        "ch4_active" => Emitter::Ch4Active,
        "ch4_freq_cnt" => Emitter::Ch4FreqCnt,
        "ch4_env_vol" => Emitter::Ch4EnvVol,
        "ch4_lfsr" => Emitter::Ch4Lfsr,
        "ch4_len_cnt" => Emitter::Ch4LenCnt,
        // Pixel output
        "pix" => Emitter::Pix,
        "pix_x" => Emitter::PpuPixX,
        // VRAM writes
        "vram_addr" => Emitter::VramAddr,
        "vram_data" => Emitter::VramData,
        // Wave RAM writes
        "wave_addr" => Emitter::WaveAddr,
        "wave_data" => Emitter::WaveData,
        // PPU internal
        "bgw_fifo_a" => Emitter::PpuInternal(PpuField::BgwFifoA),
        "bgw_fifo_b" => Emitter::PpuInternal(PpuField::BgwFifoB),
        "spr_fifo_a" => Emitter::PpuInternal(PpuField::SprFifoA),
        "spr_fifo_b" => Emitter::PpuInternal(PpuField::SprFifoB),
        "mask_pipe" => Emitter::PpuInternal(PpuField::MaskPipe),
        "pal_pipe" => Emitter::PpuInternal(PpuField::PalPipe),
        "tfetch_state" => Emitter::PpuInternal(PpuField::TfetchState),
        "sfetch_state" => Emitter::PpuInternal(PpuField::SfetchState),
        "tile_temp_a" => Emitter::PpuInternal(PpuField::TileTempA),
        "tile_temp_b" => Emitter::PpuInternal(PpuField::TileTempB),
        "pix_count" => Emitter::PpuInternal(PpuField::PixCount),
        "sprite_count" => Emitter::PpuInternal(PpuField::SpriteCount),
        "scan_count" => Emitter::PpuInternal(PpuField::ScanCount),
        "rendering" => Emitter::PpuInternal(PpuField::Rendering),
        "win_mode" => Emitter::PpuInternal(PpuField::WinMode),
        _ => {
            // OAM sprite fields: oam0_x, oam3_attr, etc.
            if let Some(rest) = field.strip_prefix("oam") {
                if let Some((idx_str, suffix)) = rest.split_once('_') {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        if idx < 10 {
                            let component = match suffix {
                                "x" => OamComponent::X,
                                "id" => OamComponent::Id,
                                "attr" => OamComponent::Attr,
                                _ => return Emitter::Unknown(FieldType::UInt8, false),
                            };
                            return Emitter::PpuInternal(PpuField::Oam { index: idx, component });
                        }
                    }
                }
            }
            // IO register
            if let Some(&(_, addr)) = IO_FIELDS.iter().find(|(name, _)| *name == field) {
                return Emitter::IoRead(addr);
            }
            // Memory-mapped field
            if let Some(&addr) = memory.get(field) {
                return Emitter::MemRead(addr);
            }
            // Unknown
            Emitter::Unknown(field_type(field), field_nullable(field))
        }
    }
}

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

        // Resolve all fields to emitters at creation time
        let mut emitters = Vec::with_capacity(profile.fields.len());
        let mut needs_ppu_snapshot = false;

        for (col, field) in profile.fields.iter().enumerate() {
            let emitter = resolve_emitter(field, &profile.memory);
            if matches!(emitter, Emitter::PpuInternal(_) | Emitter::PpuPixX) {
                needs_ppu_snapshot = true;
            }
            emitters.push(ResolvedField { col, emitter });
        }

        Ok(Self {
            writer,
            emitters,
            dot_count: 0,
            trigger,
            needs_ppu_snapshot,
            pix_buffer: String::new(),
            vram_write_addr: 0,
            vram_write_data: 0,
            wave_write_addr: 0,
            wave_write_data: 0,
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

    pub fn push_wave_write(&mut self, addr: u16, data: u8) {
        self.wave_write_addr = addr;
        self.wave_write_data = data;
    }

    pub fn capture(&mut self, gb: &GameBoy) -> Result<(), gbtrace::Error> {
        let ppu_snap = if self.needs_ppu_snapshot {
            gb.ppu().trace_snapshot()
        } else {
            None
        };

        let channels = gb.audio().channels();
        let pix_buffer = &self.pix_buffer;
        let vram_write_addr = self.vram_write_addr;
        let vram_write_data = self.vram_write_data;
        let wave_write_addr = self.wave_write_addr;
        let wave_write_data = self.wave_write_data;
        let w = &mut self.writer;

        for rf in &self.emitters {
            let col = rf.col;
            match &rf.emitter {
                // CPU
                Emitter::CpuA => w.set_u8(col, gb.cpu().a),
                Emitter::CpuF => w.set_u8(col, gb.cpu().flags.bits()),
                Emitter::CpuB => w.set_u8(col, gb.cpu().b),
                Emitter::CpuC => w.set_u8(col, gb.cpu().c),
                Emitter::CpuD => w.set_u8(col, gb.cpu().d),
                Emitter::CpuE => w.set_u8(col, gb.cpu().e),
                Emitter::CpuH => w.set_u8(col, gb.cpu().h),
                Emitter::CpuL => w.set_u8(col, gb.cpu().l),
                Emitter::CpuSp => w.set_u16(col, gb.cpu().stack_pointer),
                Emitter::CpuPc => w.set_u16(col, gb.cpu().instruction_pc),
                Emitter::CpuIme => w.set_bool(col, gb.cpu().interrupts_enabled()),
                // IO / memory reads
                Emitter::IoRead(addr) | Emitter::MemRead(addr) => {
                    w.set_u8(col, gb.peek(*addr));
                }
                // APU internal — channel 1
                Emitter::Ch1Active => w.set_bool(col, channels.ch1.enabled.enabled),
                Emitter::Ch1FreqCnt => w.set_u16(col, channels.ch1.frequency_timer),
                Emitter::Ch1EnvVol => w.set_u8(col, channels.ch1.current_volume),
                Emitter::Ch1Phase => w.set_u8(col, channels.ch1.wave_duty_position),
                Emitter::Ch1SweepShadow => w.set_u16(col, channels.ch1.shadow_frequency),
                Emitter::Ch1LenCnt => w.set_u8(col, channels.ch1.length_counter as u8),
                // APU internal — channel 2
                Emitter::Ch2Active => w.set_bool(col, channels.ch2.enabled.enabled),
                Emitter::Ch2FreqCnt => w.set_u16(col, channels.ch2.frequency_timer),
                Emitter::Ch2EnvVol => w.set_u8(col, channels.ch2.current_volume),
                Emitter::Ch2Phase => w.set_u8(col, channels.ch2.wave_duty_position),
                Emitter::Ch2LenCnt => w.set_u8(col, channels.ch2.length_counter as u8),
                // APU internal — channel 3
                Emitter::Ch3Active => w.set_bool(col, channels.ch3.enabled.enabled),
                Emitter::Ch3FreqCnt => w.set_u16(col, channels.ch3.frequency_timer),
                Emitter::Ch3WaveIdx => w.set_u8(col, channels.ch3.wave_position),
                Emitter::Ch3Sample => {
                    let byte = channels.ch3.ram[channels.ch3.wave_position as usize / 2];
                    let nibble = if channels.ch3.wave_position % 2 == 0 { byte >> 4 } else { byte & 0x0F };
                    w.set_u8(col, nibble);
                }
                Emitter::Ch3LenCnt => w.set_u8(col, channels.ch3.length_counter as u8),
                // APU internal — channel 4
                Emitter::Ch4Active => w.set_bool(col, channels.ch4.enabled.enabled),
                Emitter::Ch4FreqCnt => w.set_u16(col, channels.ch4.frequency_timer),
                Emitter::Ch4EnvVol => w.set_u8(col, channels.ch4.current_volume),
                Emitter::Ch4Lfsr => w.set_u16(col, channels.ch4.lfsr),
                Emitter::Ch4LenCnt => w.set_u8(col, channels.ch4.length_counter as u8),
                // Pixel output
                Emitter::Pix => {
                    if pix_buffer.is_empty() {
                        w.set_null(col);
                    } else {
                        w.set_str(col, pix_buffer);
                    }
                }
                Emitter::PpuPixX => {
                    if let Some(snap) = &ppu_snap {
                        w.set_u8(col, snap.pix_count);
                    } else {
                        w.set_u8(col, 0);
                    }
                }
                // VRAM write tracking
                Emitter::VramAddr => {
                    if vram_write_addr != 0 {
                        w.set_u16(col, vram_write_addr);
                    } else {
                        w.set_null(col);
                    }
                }
                Emitter::VramData => {
                    if vram_write_addr != 0 {
                        w.set_u8(col, vram_write_data);
                    } else {
                        w.set_null(col);
                    }
                }
                // Wave RAM write tracking
                Emitter::WaveAddr => {
                    if wave_write_addr != 0 {
                        w.set_u16(col, wave_write_addr);
                    } else {
                        w.set_null(col);
                    }
                }
                Emitter::WaveData => {
                    if wave_write_addr != 0 {
                        w.set_u8(col, wave_write_data);
                    } else {
                        w.set_null(col);
                    }
                }
                // PPU internal
                Emitter::PpuInternal(ppu_field) => {
                    emit_ppu_field(w, col, ppu_field, &ppu_snap);
                }
                // Unknown
                Emitter::Unknown(ft, nullable) => {
                    if *nullable {
                        w.set_null(col);
                    } else {
                        match ft {
                            FieldType::Bool => w.set_bool(col, false),
                            FieldType::Str => w.set_str(col, ""),
                            _ => w.set_u8(col, 0),
                        }
                    }
                }
            }
        }

        self.pix_buffer.clear();
        self.vram_write_addr = 0;
        self.vram_write_data = 0;
        self.wave_write_addr = 0;
        self.wave_write_data = 0;

        self.writer.finish_entry()
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

fn emit_ppu_field(
    w: &mut GbtraceWriter,
    col: usize,
    field: &PpuField,
    snap: &Option<PpuTraceSnapshot>,
) {
    let snap = match snap {
        Some(s) => s,
        None => {
            match field {
                PpuField::Rendering | PpuField::WinMode => w.set_bool(col, false),
                _ => w.set_u8(col, 0),
            }
            return;
        }
    };

    match field {
        PpuField::Oam { index, component } => {
            let val = match component {
                OamComponent::X => snap.sprite_x[*index],
                OamComponent::Id => snap.sprite_id[*index],
                OamComponent::Attr => snap.sprite_attr[*index],
            };
            w.set_u8(col, val);
        }
        PpuField::BgwFifoA => w.set_u8(col, snap.bgw_fifo_a),
        PpuField::BgwFifoB => w.set_u8(col, snap.bgw_fifo_b),
        PpuField::SprFifoA => w.set_u8(col, snap.spr_fifo_a),
        PpuField::SprFifoB => w.set_u8(col, snap.spr_fifo_b),
        PpuField::MaskPipe => w.set_u8(col, snap.mask_pipe),
        PpuField::PalPipe => w.set_u8(col, snap.pal_pipe),
        PpuField::TfetchState => w.set_u8(col, snap.tfetch_state),
        PpuField::SfetchState => w.set_u8(col, snap.sfetch_state),
        PpuField::TileTempA => w.set_u8(col, snap.tile_temp_a),
        PpuField::TileTempB => w.set_u8(col, snap.tile_temp_b),
        PpuField::PixCount => w.set_u8(col, snap.pix_count),
        PpuField::SpriteCount => w.set_u8(col, snap.sprite_count),
        PpuField::ScanCount => w.set_u8(col, snap.scan_count),
        PpuField::Rendering => w.set_bool(col, snap.rendering),
        PpuField::WinMode => w.set_bool(col, snap.win_mode),
    }
}
