//! Reading the console into the per-subsystem boundary snapshot structs.

use super::{
    ApuSnapshot, CpuSnapshot, DmaSnapshot, Mbc6State, Mbc7State, MbcSnapshot, PpuSnapshot, RtcRegs,
    SerialSnapshot, TimerSnapshot, clock_register_code,
};
use crate::Console;
use crate::cartridge::mbc::Mbc;
use crate::cpu::HaltState;

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
