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
    /// MBC3: `Some(0..=4)` when a clock register is mapped at `$A000` instead of
    /// RAM (index Seconds..DayUpper); `None` when RAM is mapped. Absent for
    /// every other mapper.
    pub clock_register: Option<u8>,
    /// MBC3 real-time clock, on carts that carry one.
    pub rtc: Option<RtcRegs>,
    /// MBC6's second ROM/RAM half and flash latches.
    pub mbc6: Option<Mbc6State>,
    /// MBC7's split enables, accelerometer latch, and EEPROM write-enable.
    pub mbc7: Option<Mbc7State>,
}

/// MBC3 real-time-clock register file as saved: the live counters ($08-$0C), the
/// latched copies a $6000 sequence froze, and the arm flag. RTCDH (day-upper)
/// bit 6 is the halt, bit 7 the sticky day-counter carry.
#[derive(Clone, Copy)]
pub struct RtcRegs {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day_lower: u8,
    pub day_upper: u8,
    pub latched_seconds: u8,
    pub latched_minutes: u8,
    pub latched_hours: u8,
    pub latched_day_lower: u8,
    pub latched_day_upper: u8,
    pub latch_ready: bool,
}

/// MBC6's second switchable half and flash state — the fields the shared
/// rom_bank/ram_bank/ram_enabled triple does not carry.
#[derive(Clone, Copy)]
pub struct Mbc6State {
    pub rom_bank_b: u8,
    pub ram_bank_b: u8,
    pub rom_a_flash: bool,
    pub rom_b_flash: bool,
    pub flash_enabled: bool,
}

/// MBC7's two independent RAM-enable latches, the latched accelerometer axes,
/// and the EEPROM write-enable.
#[derive(Clone, Copy)]
pub struct Mbc7State {
    pub ram_enabled_1: bool,
    pub ram_enabled_2: bool,
    pub accel_x: u16,
    pub accel_y: u16,
    pub write_enabled: bool,
}

/// The clock register a $4000-$5FFF select code names, as a save-state index.
fn clock_register_code(register: crate::cartridge::mbc::mbc3::ClockRegister) -> u8 {
    use crate::cartridge::mbc::mbc3::ClockRegister::*;
    match register {
        Seconds => 0,
        Minutes => 1,
        Hours => 2,
        DayLower => 3,
        DayUpper => 4,
    }
}

/// The clock register a saved index names; out-of-range falls back to Seconds.
fn clock_register_from_code(code: u8) -> crate::cartridge::mbc::mbc3::ClockRegister {
    use crate::cartridge::mbc::mbc3::ClockRegister::*;
    match code {
        1 => Minutes,
        2 => Hours,
        3 => DayLower,
        4 => DayUpper,
        _ => Seconds,
    }
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
    use crate::cartridge::mbc::mbc3::Mapped;

    let mbc = gb.cartridge().mbc();
    let base =
        |mbc_type: &str, rom_bank: u16, ram_bank: u8, ram_enabled: bool, mode: u8| MbcSnapshot {
            mbc_type: mbc_type.into(),
            rom_bank,
            ram_bank,
            ram_enabled,
            mode,
            clock_register: None,
            rtc: None,
            mbc6: None,
            mbc7: None,
        };
    match mbc {
        Mbc::NoMbc(_) => base("none", 1, 0, false, 0),
        Mbc::Mbc1(m) => base(
            "mbc1",
            m.bank as u16,
            m.ram_bank,
            m.ram_enabled,
            m.mode1 as u8,
        ),
        Mbc::Mbc2(m) => base("mbc2", m.bank as u16, 0, m.ram_enabled, 0),
        Mbc::Mbc3(m) => {
            let (ram_bank, clock_register) = match m.mapped {
                Mapped::Ram(bank) => (bank, None),
                Mapped::Clock(register) => (0, Some(clock_register_code(register))),
            };
            MbcSnapshot {
                clock_register,
                rtc: m.clock.as_ref().map(|clock| RtcRegs {
                    seconds: clock.registers.seconds,
                    minutes: clock.registers.minutes,
                    hours: clock.registers.hours,
                    day_lower: clock.registers.days_lower,
                    day_upper: clock.registers.days_upper,
                    latched_seconds: clock.latched.seconds,
                    latched_minutes: clock.latched.minutes,
                    latched_hours: clock.latched.hours,
                    latched_day_lower: clock.latched.days_lower,
                    latched_day_upper: clock.latched.days_upper,
                    latch_ready: clock.latch_ready,
                }),
                ..base("mbc3", m.bank as u16, ram_bank, m.ram_and_clock_enabled, 0)
            }
        }
        Mbc::Mbc5(m) => base(
            "mbc5",
            m.rom_bank,
            m.ram_bank,
            m.ram_enabled,
            m.rumble as u8,
        ),
        Mbc::Mbc6(m) => MbcSnapshot {
            mbc6: Some(Mbc6State {
                rom_bank_b: m.rom_bank_b,
                ram_bank_b: m.ram_bank_b,
                rom_a_flash: m.rom_bank_a_flash,
                rom_b_flash: m.rom_bank_b_flash,
                flash_enabled: m.flash_enabled,
            }),
            ..base("mbc6", m.rom_bank_a as u16, m.ram_bank_a, m.ram_enabled, 0)
        },
        Mbc::Mbc7(m) => MbcSnapshot {
            mbc7: Some(Mbc7State {
                ram_enabled_1: m.ram_enabled_1,
                ram_enabled_2: m.ram_enabled_2,
                accel_x: m.accel_x,
                accel_y: m.accel_y,
                write_enabled: m.eeprom.write_enabled,
            }),
            ..base(
                "mbc7",
                m.rom_bank as u16,
                0,
                m.ram_enabled_1 && m.ram_enabled_2,
                0,
            )
        },
        Mbc::Huc1(m) => base(
            "huc1",
            m.rom_bank as u16,
            m.ram_bank,
            false,
            m.ir_mode as u8,
        ),
        Mbc::Huc3(m) => base("huc3", m.rom_bank as u16, m.ram_bank, true, 0),
        Mbc::DbzTrans(m) => base("dbz_trans", m.rom_bank, m.ram_bank, m.ram_enabled, 0),
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
    // Mapper-specific latches beyond the shared quadruple, set only for the
    // mapper that holds them (the schema fields are nullable — a mapper without
    // them omits them).
    if let Some(code) = mbc.clock_register {
        r.set("mbc3_clock_sel", code);
    }
    if let Some(rtc) = mbc.rtc {
        r.set("rtc_seconds", rtc.seconds)
            .set("rtc_minutes", rtc.minutes)
            .set("rtc_hours", rtc.hours)
            .set("rtc_day_lower", rtc.day_lower)
            .set("rtc_day_upper", rtc.day_upper)
            .set("rtc_latched_seconds", rtc.latched_seconds)
            .set("rtc_latched_minutes", rtc.latched_minutes)
            .set("rtc_latched_hours", rtc.latched_hours)
            .set("rtc_latched_day_lower", rtc.latched_day_lower)
            .set("rtc_latched_day_upper", rtc.latched_day_upper)
            .set("rtc_latch_ready", rtc.latch_ready);
    }
    if let Some(m6) = mbc.mbc6 {
        r.set("mbc6_rom_bank_b", m6.rom_bank_b)
            .set("mbc6_ram_bank_b", m6.ram_bank_b)
            .set("mbc6_rom_a_flash", m6.rom_a_flash)
            .set("mbc6_rom_b_flash", m6.rom_b_flash)
            .set("mbc6_flash_enabled", m6.flash_enabled);
    }
    if let Some(m7) = mbc.mbc7 {
        r.set("mbc7_ram_enabled_1", m7.ram_enabled_1)
            .set("mbc7_ram_enabled_2", m7.ram_enabled_2)
            .set("mbc7_accel_x", m7.accel_x)
            .set("mbc7_accel_y", m7.accel_y)
            .set("mbc7_write_enabled", m7.write_enabled);
    }
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

/// A nullable u8 field: `None` when the record omits it or carries it as null.
fn opt_u8(record: &StateRecord, name: &str) -> Result<Option<u8>, StateError> {
    match record.get(name) {
        Some(StateValue::Int(v)) if *v <= u8::MAX as u32 => Ok(Some(*v as u8)),
        Some(StateValue::Null) | None => Ok(None),
        _ => Err(StateError::Corrupt),
    }
}

/// Parse the MBC3 real-time clock, present only when the record carries it.
fn parse_rtc(record: &StateRecord) -> Result<Option<RtcRegs>, StateError> {
    if opt_u8(record, "rtc_seconds")?.is_none() {
        return Ok(None);
    }
    Ok(Some(RtcRegs {
        seconds: u8_of(record, "rtc_seconds")?,
        minutes: u8_of(record, "rtc_minutes")?,
        hours: u8_of(record, "rtc_hours")?,
        day_lower: u8_of(record, "rtc_day_lower")?,
        day_upper: u8_of(record, "rtc_day_upper")?,
        latched_seconds: u8_of(record, "rtc_latched_seconds")?,
        latched_minutes: u8_of(record, "rtc_latched_minutes")?,
        latched_hours: u8_of(record, "rtc_latched_hours")?,
        latched_day_lower: u8_of(record, "rtc_latched_day_lower")?,
        latched_day_upper: u8_of(record, "rtc_latched_day_upper")?,
        latch_ready: bool_of(record, "rtc_latch_ready")?,
    }))
}

/// Parse MBC6's second-half/flash state, present only for an MBC6 save.
fn parse_mbc6(record: &StateRecord) -> Result<Option<Mbc6State>, StateError> {
    if opt_u8(record, "mbc6_rom_bank_b")?.is_none() {
        return Ok(None);
    }
    Ok(Some(Mbc6State {
        rom_bank_b: u8_of(record, "mbc6_rom_bank_b")?,
        ram_bank_b: u8_of(record, "mbc6_ram_bank_b")?,
        rom_a_flash: bool_of(record, "mbc6_rom_a_flash")?,
        rom_b_flash: bool_of(record, "mbc6_rom_b_flash")?,
        flash_enabled: bool_of(record, "mbc6_flash_enabled")?,
    }))
}

/// Parse MBC7's latches, present only for an MBC7 save.
fn parse_mbc7(record: &StateRecord) -> Result<Option<Mbc7State>, StateError> {
    match record.get("mbc7_ram_enabled_1") {
        Some(StateValue::Bool(_)) => {}
        _ => return Ok(None),
    }
    Ok(Some(Mbc7State {
        ram_enabled_1: bool_of(record, "mbc7_ram_enabled_1")?,
        ram_enabled_2: bool_of(record, "mbc7_ram_enabled_2")?,
        accel_x: u16_of(record, "mbc7_accel_x")?,
        accel_y: u16_of(record, "mbc7_accel_y")?,
        write_enabled: bool_of(record, "mbc7_write_enabled")?,
    }))
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
        clock_register: opt_u8(record, "mbc3_clock_sel")?,
        rtc: parse_rtc(record)?,
        mbc6: parse_mbc6(record)?,
        mbc7: parse_mbc7(record)?,
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
            return Err(StateError::NotAtBoundary);
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
            // A linear all-banks raw restore: independent of the mapper's enable
            // latch and bank window, so it neither overflows the 16-bit bus
            // address nor drops banks past the one currently mapped.
            self.chassis.external.cartridge.restore_ram(cart_ram);
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
    use crate::cartridge::mbc::mbc3::{ClockRegisters, Mapped};
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
            // Reseat the $A000 window's RAM-bank-vs-clock selection, then the
            // clock register file itself.
            m.mapped = match snap.clock_register {
                Some(code) => Mapped::Clock(clock_register_from_code(code)),
                None => Mapped::Ram(snap.ram_bank),
            };
            if let (Some(clock), Some(rtc)) = (m.clock.as_mut(), snap.rtc.as_ref()) {
                clock.registers = ClockRegisters {
                    seconds: rtc.seconds,
                    minutes: rtc.minutes,
                    hours: rtc.hours,
                    days_lower: rtc.day_lower,
                    days_upper: rtc.day_upper,
                };
                clock.latched = ClockRegisters {
                    seconds: rtc.latched_seconds,
                    minutes: rtc.latched_minutes,
                    hours: rtc.latched_hours,
                    days_lower: rtc.latched_day_lower,
                    days_upper: rtc.latched_day_upper,
                };
                clock.latch_ready = rtc.latch_ready;
            }
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
            if let Some(x) = &snap.mbc6 {
                m.rom_bank_b = x.rom_bank_b;
                m.ram_bank_b = x.ram_bank_b;
                m.rom_bank_a_flash = x.rom_a_flash;
                m.rom_bank_b_flash = x.rom_b_flash;
                m.flash_enabled = x.flash_enabled;
            }
        }
        Mbc::Mbc7(m) => {
            m.rom_bank = snap.rom_bank as u8;
            match &snap.mbc7 {
                Some(x) => {
                    m.ram_enabled_1 = x.ram_enabled_1;
                    m.ram_enabled_2 = x.ram_enabled_2;
                    m.accel_x = x.accel_x;
                    m.accel_y = x.accel_y;
                    m.eeprom.write_enabled = x.write_enabled;
                }
                None => {
                    m.ram_enabled_1 = snap.ram_enabled;
                    m.ram_enabled_2 = snap.ram_enabled;
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameBoy;
    use crate::cartridge::Cartridge;
    use crate::cartridge::mbc::mbc3::Mapped;

    /// A synthetic MBC3 ROM: header cartridge-type and RAM-size codes, a valid
    /// entry, everything else NOP.
    fn mbc3_rom(cart_type: u8, ram_code: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]); // JP $0150
        rom[0x147] = cart_type;
        rom[0x149] = ram_code;
        rom
    }

    /// Capture a console's record and its owned memory spans, as the save path
    /// does.
    fn capture(console: &GameBoy) -> (StateRecord, Vec<(String, Vec<u8>)>) {
        let record = read_shared_record(console);
        let memory = capture_memory(console)
            .into_iter()
            .map(|(name, data)| (name.to_owned(), data))
            .collect();
        (record, memory)
    }

    // ── Finding 1: linear cart-RAM restore ───────────────────────────

    #[test]
    fn mbc3_32k_ram_round_trips_across_all_banks() {
        // 32 KiB (four 8 KiB banks), each stamped distinctly. The old restore
        // replayed this through the $A000 window and panicked past bank 2 (the
        // 16-bit bus address overflowed) while dropping every unmapped bank.
        let mut save = vec![0u8; 4 * 8 * 1024];
        for (bank, chunk) in save.chunks_mut(8 * 1024).enumerate() {
            chunk.fill(0x10 + bank as u8);
            chunk[0] = 0xA0 + bank as u8;
            *chunk.last_mut().unwrap() = 0xB0 + bank as u8;
        }
        let rom = mbc3_rom(0x13, 3); // MBC3+RAM+BATTERY, 32 KiB
        let source = GameBoy::new(Cartridge::new(rom.clone(), Some(save.clone())), None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore succeeds without panicking");

        for (offset, &byte) in save.iter().enumerate() {
            assert_eq!(
                target.cartridge().peek_ram(offset),
                byte,
                "cart RAM byte {offset:#x} (bank {}) diverged",
                offset / (8 * 1024)
            );
        }
    }

    #[test]
    fn cart_ram_restores_even_when_disabled_at_load() {
        // The source's RAM was never enabled (loaded straight from a battery
        // save), and the fresh target has RAM disabled too — the raw restore is
        // enable-independent, so the bytes still land.
        let mut save = vec![0u8; 8 * 1024];
        save[0x100] = 0x5A;
        let rom = mbc3_rom(0x13, 2); // MBC3+RAM+BATTERY, 8 KiB
        let source = GameBoy::new(Cartridge::new(rom.clone(), Some(save)), None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        // The fresh target has cartridge RAM disabled.
        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => assert!(!m.ram_and_clock_enabled),
            _ => panic!("expected MBC3"),
        }
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");
        assert_eq!(target.cartridge().peek_ram(0x100), 0x5A);
    }

    // ── Finding 4: MBC3 mapped selection + RTC ───────────────────────

    #[test]
    fn mbc3_restore_lands_on_saved_ram_bank_over_a_live_clock() {
        let rom = mbc3_rom(0x10, 3); // MBC3+TIMER+RAM+BATTERY (carries a clock)

        // Source: RAM bank 2 mapped and enabled, a marker written to it.
        let mut src_cart = Cartridge::new(rom.clone(), None);
        src_cart.write(0x0000, 0x0A); // enable RAM + clock
        src_cart.write(0x4000, 0x02); // map RAM bank 2
        src_cart.write(0xA000, 0x77); // marker in bank 2
        let source = GameBoy::new(src_cart, None);
        let (record, memory) = capture(&source);

        // Target: currently on the clock register, not RAM.
        let mut tgt_cart = Cartridge::new(rom, None);
        tgt_cart.write(0x0000, 0x0A);
        tgt_cart.write(0x4000, 0x08); // map the seconds clock register
        let mut target = GameBoy::new(tgt_cart, None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");

        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => assert!(
                matches!(m.mapped, Mapped::Ram(2)),
                "restore should reseat RAM bank 2 over the live clock mapping"
            ),
            _ => panic!("expected MBC3"),
        }
        assert_eq!(
            target.cartridge().read(0xA000),
            0x77,
            "bank-2 marker reads back"
        );
    }

    #[test]
    fn mbc3_rtc_registers_round_trip() {
        let rom = mbc3_rom(0x10, 2); // MBC3+TIMER+RAM+BATTERY
        let mut src_cart = Cartridge::new(rom.clone(), None);
        src_cart.write(0x0000, 0x0A);
        src_cart.write(0x4000, 0x08); // map seconds
        src_cart.write(0xA000, 41); // seconds := 41
        src_cart.write(0x4000, 0x0A); // map hours
        src_cart.write(0xA000, 7); // hours := 7
        let source = GameBoy::new(src_cart, None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");
        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => {
                let clock = m.clock.as_ref().expect("clock present");
                assert_eq!(clock.registers.seconds, 41);
                assert_eq!(clock.registers.hours, 7);
            }
            _ => panic!("expected MBC3"),
        }
    }
}
