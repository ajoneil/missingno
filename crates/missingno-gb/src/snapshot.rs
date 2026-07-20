//! The DMG save-state bridge: it maps the Game Boy's internal state onto the
//! hardware-named [`SystemStateSchema`] and back. Capture reads the console into
//! a [`StateRecord`] keyed by the schema's field names; restore parses a record
//! into per-subsystem snapshot structs and rebuilds the console in place at an
//! instruction boundary.
//!
//! The snapshot structs are missingno's internal boundary DTO — the input the
//! per-component `from_snapshot` constructors already consume. The mapping
//! between a struct's emulator-shaped field and its hardware-named schema field
//! (halt_state → cpu_mode, ei_delay → ime_enable_pending, dot_position → lx) is
//! the bridge's substance; the format sees only the hardware quantity.
//!
//! Restore is boundary-faithful (Tier-2a): the pixel-pipeline latches are not
//! round-tripped — at a frame/instruction boundary they reconstruct from the
//! pipeline's boundary defaults — so the schema marks those fields nullable and
//! this bridge omits them.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::state_file::StateFrame;
use missingno_core::system::StateError;

use crate::audio::Audio;
use crate::cartridge::mbc::Mbc;
use crate::cpu::HaltState;
use crate::interrupts::InterruptFlags;
use crate::{Console, Model, ScreenBuffer};

// ── Per-subsystem boundary snapshot structs ──────────────────────
//
// missingno's internal boundary DTO. Each per-component `from_snapshot`
// constructor consumes its struct; capture fills them from the console.

pub struct CpuSnapshot {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    pub if_: u8,
    pub ie: u8,
    /// Run state code: 0 running, 1 halting, 2 halted/stopped, 3 locked.
    pub halt_state: u8,
    /// EI's deferred enable in flight (1) or not (0).
    pub ei_delay: u8,
    pub halt_bug: bool,
}

pub struct PpuSnapshot {
    pub lcdc: u8,
    pub stat: u8,
    pub ly: u8,
    pub lyc: u8,
    pub scy: u8,
    pub scx: u8,
    pub wy: u8,
    pub wx: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub dma: u8,
    pub dot_position: u8,
    pub stat_line_was_high: bool,
    pub window_line_counter: u8,
}

pub struct ApuSnapshot {
    pub master_vol: u8,
    pub sound_pan: u8,
    pub sound_on: u8,
    pub ch1_sweep: u8,
    pub ch1_duty_len: u8,
    pub ch1_vol_env: u8,
    pub ch1_freq_lo: u8,
    pub ch1_freq_hi: u8,
    pub ch2_duty_len: u8,
    pub ch2_vol_env: u8,
    pub ch2_freq_lo: u8,
    pub ch2_freq_hi: u8,
    pub ch3_dac: u8,
    pub ch3_len: u8,
    pub ch3_vol: u8,
    pub ch3_freq_lo: u8,
    pub ch3_freq_hi: u8,
    pub ch4_len: u8,
    pub ch4_vol_env: u8,
    pub ch4_freq: u8,
    pub ch4_control: u8,
    pub frame_sequencer_step: u8,
    pub prev_div_apu_bit: bool,
    pub ch1_period: u16,
    pub ch1_envelope_timer: u8,
    pub ch1_sweep_timer: u8,
    pub ch1_sweep_enabled: bool,
    pub ch1_sweep_negate_used: bool,
    pub ch1_length_enabled: bool,
    pub ch2_period: u16,
    pub ch2_envelope_timer: u8,
    pub ch2_length_enabled: bool,
    pub ch3_period: u16,
    pub ch3_length_enabled: bool,
    pub ch4_envelope_timer: u8,
    pub ch4_length_enabled: bool,
}

pub struct TimerSnapshot {
    pub div: u8,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
    pub internal_counter: u16,
    pub overflow_pending: bool,
    pub reloading: bool,
}

pub struct DmaSnapshot {
    pub active: bool,
    pub source: u16,
    pub byte_index: u8,
    pub delay_remaining: u8,
}

pub struct SerialSnapshot {
    pub sb: u8,
    pub sc: u8,
    pub bits_remaining: u8,
    pub shift_clock: bool,
}

pub struct MbcSnapshot {
    pub mbc_type: String,
    pub rom_bank: u16,
    pub ram_bank: u8,
    pub ram_enabled: bool,
    pub mode: u8,
}

/// All parsed subsystem snapshots plus the named RAM regions — the input to an
/// in-place restore.
pub struct Snapshot {
    pub cpu: CpuSnapshot,
    pub ppu: PpuSnapshot,
    pub apu: ApuSnapshot,
    pub timer: TimerSnapshot,
    pub dma: DmaSnapshot,
    pub serial: SerialSnapshot,
    pub mbc: MbcSnapshot,
    pub memory: Vec<(String, Vec<u8>)>,
}

// ── Capture: console → structs ───────────────────────────────────

pub fn capture_cpu<M: crate::Model>(gb: &Console<M>) -> CpuSnapshot {
    let cpu = gb.cpu();
    CpuSnapshot {
        a: cpu.a,
        f: cpu.flags.bits(),
        b: cpu.b,
        c: cpu.c,
        d: cpu.d,
        e: cpu.e,
        h: cpu.h,
        l: cpu.l,
        sp: cpu.stack_pointer,
        pc: cpu.pc,
        ime: cpu.interrupts_enabled(),
        if_: gb.interrupts().requested.bits(),
        ie: gb.interrupts().enabled.bits(),
        halt_state: match cpu.halt.state {
            HaltState::Running => 0,
            HaltState::Halting => 1,
            HaltState::Halted | HaltState::Stopped => 2,
            HaltState::Locked => 3,
        },
        ei_delay: if cpu.irq.ime_delay && !cpu.interrupts_enabled() {
            1
        } else {
            0
        },
        halt_bug: cpu.halt.bug,
    }
}

pub fn capture_ppu<M: crate::Model>(gb: &Console<M>) -> PpuSnapshot {
    let ppu = gb.ppu();
    PpuSnapshot {
        lcdc: ppu.read_register(crate::ppu::Register::Control),
        stat: ppu.read_register(crate::ppu::Register::Status),
        ly: ppu.read_register(crate::ppu::Register::CurrentScanline),
        lyc: ppu.read_register(crate::ppu::Register::InterruptOnScanline),
        scy: ppu.read_register(crate::ppu::Register::BackgroundViewportY),
        scx: ppu.read_register(crate::ppu::Register::BackgroundViewportX),
        wy: ppu.read_register(crate::ppu::Register::WindowY),
        wx: ppu.read_register(crate::ppu::Register::WindowX),
        bgp: ppu.read_register(crate::ppu::Register::BackgroundPalette),
        obp0: ppu.read_register(crate::ppu::Register::Sprite0Palette),
        obp1: ppu.read_register(crate::ppu::Register::Sprite1Palette),
        dma: gb.dma().source_register(),
        dot_position: ppu.lx(),
        stat_line_was_high: ppu.stat_line_was_high(),
        window_line_counter: ppu.window_line_counter().unwrap_or(0),
    }
}

pub fn capture_apu<M: crate::Model>(gb: &Console<M>) -> ApuSnapshot {
    let audio = gb.audio();
    let ch = audio.channels();
    ApuSnapshot {
        master_vol: audio.nr50,
        sound_pan: gb.peek(0xFF25),
        sound_on: gb.peek(0xFF26),

        ch1_sweep: ch.ch1.sweep.0,
        ch1_duty_len: ch.ch1.waveform_and_initial_length.0,
        ch1_vol_env: ch.ch1.volume_and_envelope.0,
        ch1_freq_lo: ch.ch1.period.0 as u8,
        ch1_freq_hi: (ch.ch1.period.0 >> 8) as u8 | if ch.ch1.length.enabled { 0x40 } else { 0 },

        ch2_duty_len: ch.ch2.waveform_and_initial_length.0,
        ch2_vol_env: ch.ch2.volume_and_envelope.0,
        ch2_freq_lo: ch.ch2.period.0 as u8,
        ch2_freq_hi: (ch.ch2.period.0 >> 8) as u8 | if ch.ch2.length.enabled { 0x40 } else { 0 },

        ch3_dac: if ch.ch3.dac_enabled { 0x80 } else { 0 },
        ch3_len: gb.peek(0xFF1B),
        ch3_vol: ch.ch3.volume.0,
        ch3_freq_lo: ch.ch3.period.0 as u8,
        ch3_freq_hi: (ch.ch3.period.0 >> 8) as u8 | if ch.ch3.length.enabled { 0x40 } else { 0 },

        ch4_len: gb.peek(0xFF20),
        ch4_vol_env: ch.ch4.volume_and_envelope.0,
        ch4_freq: ch.ch4.frequency_and_randomness.0,
        ch4_control: if ch.ch4.length.enabled { 0x40 } else { 0 },

        frame_sequencer_step: audio.frame_sequencer_step,
        prev_div_apu_bit: audio.prev_div_apu_bit,

        ch1_period: ch.ch1.period.0,
        ch1_envelope_timer: ch.ch1.envelope.timer,
        ch1_sweep_timer: ch.ch1.sweep_timer,
        ch1_sweep_enabled: ch.ch1.sweep_enabled,
        ch1_sweep_negate_used: ch.ch1.sweep_negate_used,
        ch1_length_enabled: ch.ch1.length.enabled,

        ch2_period: ch.ch2.period.0,
        ch2_envelope_timer: ch.ch2.envelope.timer,
        ch2_length_enabled: ch.ch2.length.enabled,

        ch3_period: ch.ch3.period.0,
        ch3_length_enabled: ch.ch3.length.enabled,

        ch4_envelope_timer: ch.ch4.envelope.timer,
        ch4_length_enabled: ch.ch4.length.enabled,
    }
}

pub fn capture_timer<M: crate::Model>(gb: &Console<M>) -> TimerSnapshot {
    let t = gb.timers();
    TimerSnapshot {
        div: t.read_register(crate::timers::Register::Divider),
        tima: t.counter,
        tma: t.modulo,
        tac: t.control.0,
        internal_counter: t.internal_counter,
        overflow_pending: t.overflow_pending,
        reloading: t.reloading,
    }
}

pub fn capture_dma<M: crate::Model>(gb: &Console<M>) -> DmaSnapshot {
    let dma = gb.dma();
    if dma.dma_run() {
        DmaSnapshot {
            active: true,
            source: (dma.source_register() as u16) << 8,
            byte_index: dma.byte_index(),
            delay_remaining: 0,
        }
    } else {
        DmaSnapshot {
            active: false,
            source: 0,
            byte_index: 0,
            delay_remaining: 0,
        }
    }
}

pub fn capture_serial<M: crate::Model>(gb: &Console<M>) -> SerialSnapshot {
    let r = &gb.serial().registers;
    SerialSnapshot {
        sb: r.data,
        sc: r.control.bits(),
        bits_remaining: r.shift.bits_remaining(),
        shift_clock: r.serial_clock,
    }
}

pub fn capture_mbc<M: crate::Model>(gb: &Console<M>) -> MbcSnapshot {
    let mbc = gb.cartridge().mbc();
    match mbc {
        Mbc::NoMbc(_) => MbcSnapshot {
            mbc_type: "none".into(),
            rom_bank: 1,
            ram_bank: 0,
            ram_enabled: false,
            mode: 0,
        },
        Mbc::Mbc1(m) => MbcSnapshot {
            mbc_type: "mbc1".into(),
            rom_bank: m.bank as u16,
            ram_bank: m.ram_bank,
            ram_enabled: m.ram_enabled,
            mode: m.mode1 as u8,
        },
        Mbc::Mbc2(m) => MbcSnapshot {
            mbc_type: "mbc2".into(),
            rom_bank: m.bank as u16,
            ram_bank: 0,
            ram_enabled: m.ram_enabled,
            mode: 0,
        },
        Mbc::Mbc3(m) => MbcSnapshot {
            mbc_type: "mbc3".into(),
            rom_bank: m.bank as u16,
            ram_bank: 0,
            ram_enabled: m.ram_and_clock_enabled,
            mode: 0,
        },
        Mbc::Mbc5(m) => MbcSnapshot {
            mbc_type: "mbc5".into(),
            rom_bank: m.rom_bank,
            ram_bank: m.ram_bank,
            ram_enabled: m.ram_enabled,
            mode: m.rumble as u8,
        },
        Mbc::Mbc6(m) => MbcSnapshot {
            mbc_type: "mbc6".into(),
            rom_bank: m.rom_bank_a as u16,
            ram_bank: m.ram_bank_a,
            ram_enabled: m.ram_enabled,
            mode: 0,
        },
        Mbc::Mbc7(m) => MbcSnapshot {
            mbc_type: "mbc7".into(),
            rom_bank: m.rom_bank as u16,
            ram_bank: 0,
            ram_enabled: m.ram_enabled_1 && m.ram_enabled_2,
            mode: 0,
        },
        Mbc::Huc1(m) => MbcSnapshot {
            mbc_type: "huc1".into(),
            rom_bank: m.rom_bank as u16,
            ram_bank: m.ram_bank,
            ram_enabled: false,
            mode: m.ir_mode as u8,
        },
        Mbc::Huc3(m) => MbcSnapshot {
            mbc_type: "huc3".into(),
            rom_bank: m.rom_bank as u16,
            ram_bank: m.ram_bank,
            ram_enabled: true,
            mode: 0,
        },
        Mbc::DbzTrans(m) => MbcSnapshot {
            mbc_type: "dbz_trans".into(),
            rom_bank: m.rom_bank,
            ram_bank: m.ram_bank,
            ram_enabled: m.ram_enabled,
            mode: 0,
        },
    }
}

/// The Game Boy RAM regions a save state carries, keyed by schema span name.
pub fn capture_memory<M: crate::Model>(gb: &Console<M>) -> Vec<(&'static str, Vec<u8>)> {
    let mut regions = vec![
        ("vram", gb.peek_range(0x8000, 0x2000)),
        ("wram", gb.external_bus().work_ram.to_vec()),
        ("oam", gb.peek_range(0xFE00, 0x00A0)),
        ("wave_ram", gb.audio().channels().ch3.ram.to_vec()),
        ("hram", gb.high_ram().data().to_vec()),
    ];
    if let Some(ram) = gb.cartridge().ram()
        && !ram.is_empty()
    {
        regions.push(("cart_ram", ram));
    }
    regions
}

// ── Capture: structs → record ────────────────────────────────────

/// Read the whole console into a schema-keyed record. The pipeline latch fields
/// (nullable in the schema) are omitted — this is a boundary capture.
pub fn read_shared_record<M: crate::Model>(gb: &Console<M>) -> StateRecord {
    let cpu = capture_cpu(gb);
    let ppu = capture_ppu(gb);
    let apu = capture_apu(gb);
    let timer = capture_timer(gb);
    let dma = capture_dma(gb);
    let serial = capture_serial(gb);
    let mbc = capture_mbc(gb);

    let mut r = StateRecord::new();
    // CPU.
    r.set("a", cpu.a)
        .set("f", cpu.f)
        .set("b", cpu.b)
        .set("c", cpu.c)
        .set("d", cpu.d)
        .set("e", cpu.e)
        .set("h", cpu.h)
        .set("l", cpu.l)
        .set("sp", cpu.sp)
        .set("pc", cpu.pc)
        .set("ime", cpu.ime)
        .set("if_", cpu.if_)
        .set("ie", cpu.ie)
        .set("cpu_mode", cpu.halt_state)
        .set("ime_enable_pending", cpu.ei_delay != 0)
        .set("halt_bug", cpu.halt_bug);
    // PPU registers + deep boundary state.
    r.set("lcdc", ppu.lcdc)
        .set("stat", ppu.stat)
        .set("ly", ppu.ly)
        .set("lyc", ppu.lyc)
        .set("scy", ppu.scy)
        .set("scx", ppu.scx)
        .set("wy", ppu.wy)
        .set("wx", ppu.wx)
        .set("bgp", ppu.bgp)
        .set("obp0", ppu.obp0)
        .set("obp1", ppu.obp1)
        .set("dma", ppu.dma)
        .set("lx", ppu.dot_position)
        .set("stat_line", ppu.stat_line_was_high)
        .set("window_line_counter", ppu.window_line_counter);
    // Timer.
    r.set("div", timer.div)
        .set("tima", timer.tima)
        .set("tma", timer.tma)
        .set("tac", timer.tac)
        .set("internal_counter", timer.internal_counter)
        .set("overflow_pending", timer.overflow_pending)
        .set("reloading", timer.reloading);
    // Serial.
    r.set("sb", serial.sb)
        .set("sc", serial.sc)
        .set("serial_bits_remaining", serial.bits_remaining)
        .set("serial_clock", serial.shift_clock);
    // APU registers.
    r.set("master_vol", apu.master_vol)
        .set("sound_pan", apu.sound_pan)
        .set("sound_on", apu.sound_on)
        .set("ch1_sweep", apu.ch1_sweep)
        .set("ch1_duty_len", apu.ch1_duty_len)
        .set("ch1_vol_env", apu.ch1_vol_env)
        .set("ch1_freq_lo", apu.ch1_freq_lo)
        .set("ch1_freq_hi", apu.ch1_freq_hi)
        .set("ch2_duty_len", apu.ch2_duty_len)
        .set("ch2_vol_env", apu.ch2_vol_env)
        .set("ch2_freq_lo", apu.ch2_freq_lo)
        .set("ch2_freq_hi", apu.ch2_freq_hi)
        .set("ch3_dac", apu.ch3_dac)
        .set("ch3_len", apu.ch3_len)
        .set("ch3_vol", apu.ch3_vol)
        .set("ch3_freq_lo", apu.ch3_freq_lo)
        .set("ch3_freq_hi", apu.ch3_freq_hi)
        .set("ch4_len", apu.ch4_len)
        .set("ch4_vol_env", apu.ch4_vol_env)
        .set("ch4_freq", apu.ch4_freq)
        .set("ch4_control", apu.ch4_control);
    // APU deep boundary state.
    r.set("frame_sequencer_step", apu.frame_sequencer_step)
        .set("prev_div_apu_bit", apu.prev_div_apu_bit)
        .set("ch1_period", apu.ch1_period)
        .set("ch1_envelope_timer", apu.ch1_envelope_timer)
        .set("ch1_sweep_timer", apu.ch1_sweep_timer)
        .set("ch1_sweep_enabled", apu.ch1_sweep_enabled)
        .set("ch1_sweep_negate_used", apu.ch1_sweep_negate_used)
        .set("ch1_length_enabled", apu.ch1_length_enabled)
        .set("ch2_period", apu.ch2_period)
        .set("ch2_envelope_timer", apu.ch2_envelope_timer)
        .set("ch2_length_enabled", apu.ch2_length_enabled)
        .set("ch3_period", apu.ch3_period)
        .set("ch3_length_enabled", apu.ch3_length_enabled)
        .set("ch4_envelope_timer", apu.ch4_envelope_timer)
        .set("ch4_length_enabled", apu.ch4_length_enabled);
    // OAM DMA engine.
    r.set("dma_active", dma.active)
        .set("dma_source", dma.source)
        .set("dma_byte_index", dma.byte_index)
        .set("dma_delay", dma.delay_remaining);
    // Cartridge mapper latches.
    r.set("mbc_type", mbc.mbc_type)
        .set("rom_bank", mbc.rom_bank)
        .set("ram_bank", mbc.ram_bank)
        .set("ram_enabled", mbc.ram_enabled)
        .set("mbc_mode", mbc.mode);
    r
}

// ── Record readout helpers ───────────────────────────────────────

fn int(record: &StateRecord, name: &str) -> Result<u32, StateError> {
    match record.get(name) {
        Some(StateValue::Int(v)) => Ok(*v),
        _ => Err(StateError::Corrupt),
    }
}

fn u8_of(record: &StateRecord, name: &str) -> Result<u8, StateError> {
    Ok(int(record, name)? as u8)
}

fn u16_of(record: &StateRecord, name: &str) -> Result<u16, StateError> {
    Ok(int(record, name)? as u16)
}

fn bool_of(record: &StateRecord, name: &str) -> Result<bool, StateError> {
    match record.get(name) {
        Some(StateValue::Bool(b)) => Ok(*b),
        _ => Err(StateError::Corrupt),
    }
}

fn text_of(record: &StateRecord, name: &str) -> Result<String, StateError> {
    match record.get(name) {
        Some(StateValue::Text(t)) => Ok(t.clone()),
        _ => Err(StateError::Corrupt),
    }
}

// ── Restore: record → structs ────────────────────────────────────

/// Parse a validated record (and the file's memory spans) into the boundary
/// snapshot structs an in-place restore consumes.
pub fn parse_record(
    record: &StateRecord,
    memory: Vec<(String, Vec<u8>)>,
) -> Result<Snapshot, StateError> {
    let cpu = CpuSnapshot {
        a: u8_of(record, "a")?,
        f: u8_of(record, "f")?,
        b: u8_of(record, "b")?,
        c: u8_of(record, "c")?,
        d: u8_of(record, "d")?,
        e: u8_of(record, "e")?,
        h: u8_of(record, "h")?,
        l: u8_of(record, "l")?,
        sp: u16_of(record, "sp")?,
        pc: u16_of(record, "pc")?,
        ime: bool_of(record, "ime")?,
        if_: u8_of(record, "if_")?,
        ie: u8_of(record, "ie")?,
        halt_state: u8_of(record, "cpu_mode")?,
        ei_delay: bool_of(record, "ime_enable_pending")? as u8,
        halt_bug: bool_of(record, "halt_bug")?,
    };
    let ppu = PpuSnapshot {
        lcdc: u8_of(record, "lcdc")?,
        stat: u8_of(record, "stat")?,
        ly: u8_of(record, "ly")?,
        lyc: u8_of(record, "lyc")?,
        scy: u8_of(record, "scy")?,
        scx: u8_of(record, "scx")?,
        wy: u8_of(record, "wy")?,
        wx: u8_of(record, "wx")?,
        bgp: u8_of(record, "bgp")?,
        obp0: u8_of(record, "obp0")?,
        obp1: u8_of(record, "obp1")?,
        dma: u8_of(record, "dma")?,
        dot_position: u8_of(record, "lx")?,
        stat_line_was_high: bool_of(record, "stat_line")?,
        window_line_counter: u8_of(record, "window_line_counter")?,
    };
    let apu = ApuSnapshot {
        master_vol: u8_of(record, "master_vol")?,
        sound_pan: u8_of(record, "sound_pan")?,
        sound_on: u8_of(record, "sound_on")?,
        ch1_sweep: u8_of(record, "ch1_sweep")?,
        ch1_duty_len: u8_of(record, "ch1_duty_len")?,
        ch1_vol_env: u8_of(record, "ch1_vol_env")?,
        ch1_freq_lo: u8_of(record, "ch1_freq_lo")?,
        ch1_freq_hi: u8_of(record, "ch1_freq_hi")?,
        ch2_duty_len: u8_of(record, "ch2_duty_len")?,
        ch2_vol_env: u8_of(record, "ch2_vol_env")?,
        ch2_freq_lo: u8_of(record, "ch2_freq_lo")?,
        ch2_freq_hi: u8_of(record, "ch2_freq_hi")?,
        ch3_dac: u8_of(record, "ch3_dac")?,
        ch3_len: u8_of(record, "ch3_len")?,
        ch3_vol: u8_of(record, "ch3_vol")?,
        ch3_freq_lo: u8_of(record, "ch3_freq_lo")?,
        ch3_freq_hi: u8_of(record, "ch3_freq_hi")?,
        ch4_len: u8_of(record, "ch4_len")?,
        ch4_vol_env: u8_of(record, "ch4_vol_env")?,
        ch4_freq: u8_of(record, "ch4_freq")?,
        ch4_control: u8_of(record, "ch4_control")?,
        frame_sequencer_step: u8_of(record, "frame_sequencer_step")?,
        prev_div_apu_bit: bool_of(record, "prev_div_apu_bit")?,
        ch1_period: u16_of(record, "ch1_period")?,
        ch1_envelope_timer: u8_of(record, "ch1_envelope_timer")?,
        ch1_sweep_timer: u8_of(record, "ch1_sweep_timer")?,
        ch1_sweep_enabled: bool_of(record, "ch1_sweep_enabled")?,
        ch1_sweep_negate_used: bool_of(record, "ch1_sweep_negate_used")?,
        ch1_length_enabled: bool_of(record, "ch1_length_enabled")?,
        ch2_period: u16_of(record, "ch2_period")?,
        ch2_envelope_timer: u8_of(record, "ch2_envelope_timer")?,
        ch2_length_enabled: bool_of(record, "ch2_length_enabled")?,
        ch3_period: u16_of(record, "ch3_period")?,
        ch3_length_enabled: bool_of(record, "ch3_length_enabled")?,
        ch4_envelope_timer: u8_of(record, "ch4_envelope_timer")?,
        ch4_length_enabled: bool_of(record, "ch4_length_enabled")?,
    };
    let timer = TimerSnapshot {
        div: u8_of(record, "div")?,
        tima: u8_of(record, "tima")?,
        tma: u8_of(record, "tma")?,
        tac: u8_of(record, "tac")?,
        internal_counter: u16_of(record, "internal_counter")?,
        overflow_pending: bool_of(record, "overflow_pending")?,
        reloading: bool_of(record, "reloading")?,
    };
    let dma = DmaSnapshot {
        active: bool_of(record, "dma_active")?,
        source: u16_of(record, "dma_source")?,
        byte_index: u8_of(record, "dma_byte_index")?,
        delay_remaining: u8_of(record, "dma_delay")?,
    };
    let serial = SerialSnapshot {
        sb: u8_of(record, "sb")?,
        sc: u8_of(record, "sc")?,
        bits_remaining: u8_of(record, "serial_bits_remaining")?,
        shift_clock: bool_of(record, "serial_clock")?,
    };
    let mbc = MbcSnapshot {
        mbc_type: text_of(record, "mbc_type")?,
        rom_bank: u16_of(record, "rom_bank")?,
        ram_bank: u8_of(record, "ram_bank")?,
        ram_enabled: bool_of(record, "ram_enabled")?,
        mode: u8_of(record, "mbc_mode")?,
    };
    Ok(Snapshot {
        cpu,
        ppu,
        apu,
        timer,
        dma,
        serial,
        mbc,
        memory,
    })
}

// ── Restore: structs → console (in place) ────────────────────────

impl<M: Model> Console<M> {
    /// Restore this console in place from a validated record at an instruction
    /// boundary: the shared subsystems, the model's banked memory and register
    /// delta, and the displayed framebuffer. Errors (never panics) on a
    /// mid-instruction call or a record this model cannot faithfully restore.
    pub fn restore_boundary(
        &mut self,
        record: &StateRecord,
        memory: Vec<(String, Vec<u8>)>,
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        // A save is taken between instructions: the CPU is either about to fetch
        // or halted (waiting on an interrupt). Both are clean boundaries; a
        // mid-instruction or speed-switch-stopped console is not restorable.
        if !self.cpu().is_fetch_phase() && !self.cpu().is_halted() {
            return Err(StateError::Corrupt);
        }
        self.model.validate_boundary(record)?;
        let snapshot = parse_record(record, memory)?;
        self.restore_snapshot(&snapshot);
        self.model
            .restore_boundary_delta(&mut self.chassis, record, &snapshot.memory)?;
        // Seed the displayed screen from the saved framebuffer so the first
        // frame after a restore matches the save.
        if let Some(frame) = frame {
            self.chassis.screen.restore(&frame.data);
        }
        Ok(())
    }

    /// Rebuild the shared subsystems in place from a boundary snapshot, keeping
    /// the existing cartridge (the ROM) and re-seating every subsystem at an
    /// instruction boundary: the clock is placed at the single-speed `Rise`
    /// phase and the volatile bus/pixel state is defaulted. Model-specific state
    /// (CGB banks, palette RAM, speed) is reseated separately by
    /// [`Model::restore_boundary_delta`].
    pub fn restore_snapshot(&mut self, snap: &Snapshot) {
        use crate::memory::VramBus;
        use crate::ppu::memory::{Oam, Vram};
        use crate::ppu::model::PpuModel;

        let region = |name: &str| -> Option<&[u8]> {
            snap.memory
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, data)| data.as_slice())
        };

        let mut wave_ram = [0u8; 16];
        if let Some(data) = region("wave_ram") {
            let len = data.len().min(16);
            wave_ram[..len].copy_from_slice(&data[..len]);
        }

        // Work RAM lands where the model keeps it (DMG flat bus, CGB banks);
        // cartridge RAM lands in the existing external bus.
        if let Some(wram) = region("wram") {
            self.model
                .restore_work_ram(&mut self.chassis.external, wram);
        }
        if let Some(cart_ram) = region("cart_ram") {
            for (i, &byte) in cart_ram.iter().enumerate() {
                self.chassis
                    .external
                    .cartridge
                    .write(0xA000 + i as u16, byte);
            }
        }
        restore_mbc(&snap.mbc, self.chassis.external.cartridge.mbc_mut());

        let oam = region("oam").map(Oam::from_bytes).unwrap_or_default();

        self.chassis.cpu = crate::cpu::Cpu::from_snapshot(&snap.cpu);
        self.chassis.ppu = crate::ppu::Ppu::from_snapshot(&snap.ppu, oam);
        self.chassis.audio = Audio::from_snapshot(&snap.apu, wave_ram);
        self.chassis.timers = crate::timers::Timers::from_snapshot(&snap.timer);
        self.chassis.dma = crate::dma::Dma::from_snapshot(&snap.dma);
        // The OAM-DMA source register (FF46) reads back independently of an
        // in-flight transfer, so restore it from the captured register value.
        self.chassis.dma.set_source_register(snap.ppu.dma);
        self.chassis.serial = crate::serial_transfer::Serial::from_snapshot(&snap.serial);
        self.chassis.interrupts = {
            let mut regs = crate::interrupts::Registers::new();
            regs.enabled = InterruptFlags::from_bits_retain(snap.cpu.ie);
            regs.requested = InterruptFlags::from_bits_retain(snap.cpu.if_);
            regs
        };
        let mut vram = <<M::Ppu as PpuModel>::Vram>::default();
        if let Some(bytes) = region("vram") {
            vram.restore_image(bytes);
        }
        self.chassis.vram_bus = VramBus { vram, latch: 0xFF };
        self.chassis.high_ram = region("hram")
            .map(crate::memory::HighRam::from_bytes)
            .unwrap_or_else(crate::memory::HighRam::new);

        self.chassis.screen = M::Screen::default();
        self.chassis.bus_trace = crate::cpu_bus::BusTrace::new();
        self.chassis.clock = crate::MasterClock::new(crate::CpuDivider::One);
        self.chassis.cpu_bus = crate::cpu_bus::CpuBus::new();
        self.chassis.dma_conflict = crate::DmaConflictLatch::default();
        self.chassis.joypad = crate::joypad::Joypad::new();
    }
}

fn restore_mbc(snap: &MbcSnapshot, mbc: &mut Mbc) {
    match mbc {
        Mbc::NoMbc(_) => {}
        Mbc::Mbc1(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
            m.mode1 = snap.mode != 0;
        }
        Mbc::Mbc2(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_enabled = snap.ram_enabled;
        }
        Mbc::Mbc3(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_and_clock_enabled = snap.ram_enabled;
        }
        Mbc::Mbc5(m) => {
            m.rom_bank = snap.rom_bank;
            m.ram_bank = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
            m.rumble = snap.mode != 0;
        }
        Mbc::Mbc6(m) => {
            m.rom_bank_a = snap.rom_bank as u8;
            m.ram_bank_a = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
        }
        Mbc::Mbc7(m) => {
            m.rom_bank = snap.rom_bank as u8;
            m.ram_enabled_1 = snap.ram_enabled;
            m.ram_enabled_2 = snap.ram_enabled;
        }
        Mbc::Huc1(m) => {
            m.rom_bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
            m.ir_mode = snap.mode != 0;
        }
        Mbc::Huc3(m) => {
            m.rom_bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
        }
        Mbc::DbzTrans(m) => m.restore(snap.rom_bank, snap.ram_bank, snap.ram_enabled),
    }
}
