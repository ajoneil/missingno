//! Capture and restore gbtrace snapshot payloads from/to a GameBoy instance.
//!
//! Capture: reads emulator state into gbtrace snapshot structs.
//! Restore: each component has a `from_snapshot` constructor that builds
//! it directly from the snapshot data (no mutation after construction).

use gbtrace::format::SnapshotType;
use gbtrace::snapshot::{
    ApuSnapshot, CpuSnapshot, DmaSnapshot, MbcSnapshot, MemoryRegion, PpuSnapshot, SerialSnapshot,
    TimerSnapshot, build_memory_payload,
};

use crate::audio::Audio;
use crate::cartridge::Cartridge;
use crate::cartridge::mbc::Mbc;
use crate::cpu::HaltState;
use crate::interrupts::InterruptFlags;
use crate::{ClockPhase, GameBoy};

/// A typed snapshot payload ready to be written.
pub struct SnapshotRecord {
    pub snapshot_type: SnapshotType,
    pub payload: Vec<u8>,
}

/// Capture the full emulator state as a set of snapshot records.
///
/// The returned records are independent and self-contained — each
/// can be written into a gbtrace file or used for save state restore.
pub fn capture_snapshots(gb: &GameBoy) -> Vec<SnapshotRecord> {
    vec![
        SnapshotRecord {
            snapshot_type: SnapshotType::CpuState,
            payload: capture_cpu(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::PpuTiming,
            payload: capture_ppu(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::ApuState,
            payload: capture_apu(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::TimerState,
            payload: capture_timer(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::DmaState,
            payload: capture_dma(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::SerialState,
            payload: capture_serial(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::MbcState,
            payload: capture_mbc(gb).to_bytes(),
        },
        SnapshotRecord {
            snapshot_type: SnapshotType::Memory,
            payload: capture_memory(gb),
        },
    ]
}

pub fn capture_cpu(gb: &GameBoy) -> CpuSnapshot {
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
        pc: cpu.bus_counter,
        ime: cpu.interrupts_enabled(),
        if_: gb.interrupts().requested.bits(),
        ie: gb.interrupts().enabled.bits(),
        halt_state: match cpu.halt_state {
            HaltState::Running => 0,
            HaltState::Halting => 1,
            HaltState::Halted => 2,
        },
        // Legacy gbtrace field. The emulator no longer models EI as a
        // multi-stage pipeline — IME enables synchronously inside EI's
        // commit, and the "1-instruction delay" lives in the dispatch_active
        // chain. No persistent EI-delay state to capture here.
        ei_delay: 0,
        halt_bug: cpu.halt_bug,
    }
}

pub fn capture_ppu(gb: &GameBoy) -> PpuSnapshot {
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
        window_line_counter: 0, // TODO: only accessible mid-frame via Rendering
    }
}

pub fn capture_apu(gb: &GameBoy) -> ApuSnapshot {
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
        ch1_freq_hi: (ch.ch1.period.0 >> 8) as u8 | if ch.ch1.length_enabled { 0x40 } else { 0 },

        ch2_duty_len: ch.ch2.waveform_and_initial_length.0,
        ch2_vol_env: ch.ch2.volume_and_envelope.0,
        ch2_freq_lo: ch.ch2.period.0 as u8,
        ch2_freq_hi: (ch.ch2.period.0 >> 8) as u8 | if ch.ch2.length_enabled { 0x40 } else { 0 },

        ch3_dac: if ch.ch3.dac_enabled { 0x80 } else { 0 },
        ch3_len: gb.peek(0xFF1B),
        ch3_vol: ch.ch3.volume.0,
        ch3_freq_lo: ch.ch3.period.0 as u8,
        ch3_freq_hi: (ch.ch3.period.0 >> 8) as u8 | if ch.ch3.length_enabled { 0x40 } else { 0 },

        ch4_len: gb.peek(0xFF20),
        ch4_vol_env: ch.ch4.volume_and_envelope.0,
        ch4_freq: ch.ch4.frequency_and_randomness.0,
        ch4_control: if ch.ch4.length_enabled { 0x40 } else { 0 },

        frame_sequencer_step: audio.frame_sequencer_step,
        prev_div_apu_bit: audio.prev_div_apu_bit,

        ch1_period: ch.ch1.period.0,
        ch1_envelope_timer: ch.ch1.envelope_timer,
        ch1_sweep_timer: ch.ch1.sweep_timer,
        ch1_sweep_enabled: ch.ch1.sweep_enabled,
        ch1_sweep_negate_used: ch.ch1.sweep_negate_used,
        ch1_length_enabled: ch.ch1.length_enabled,

        ch2_period: ch.ch2.period.0,
        ch2_envelope_timer: ch.ch2.envelope_timer,
        ch2_length_enabled: ch.ch2.length_enabled,

        ch3_period: ch.ch3.period.0,
        ch3_length_enabled: ch.ch3.length_enabled,

        ch4_envelope_timer: ch.ch4.envelope_timer,
        ch4_length_enabled: ch.ch4.length_enabled,
    }
}

pub fn capture_timer(gb: &GameBoy) -> TimerSnapshot {
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

pub fn capture_dma(gb: &GameBoy) -> DmaSnapshot {
    use crate::dma::DmaDelay;
    let dma = gb.dma();
    match &dma.transfer {
        None => DmaSnapshot {
            active: false,
            source: 0,
            byte_index: 0,
            delay_remaining: 0,
        },
        Some(t) => DmaSnapshot {
            active: true,
            source: t.source,
            byte_index: t.byte_index,
            delay_remaining: match &t.delay {
                None => 0,
                Some(DmaDelay::Startup(n)) => 0x80 | n,
                Some(DmaDelay::Transfer(n)) => *n,
            },
        },
    }
}

pub fn capture_serial(gb: &GameBoy) -> SerialSnapshot {
    let s = gb.serial();
    SerialSnapshot {
        sb: s.data,
        sc: s.control.bits(),
        bits_remaining: s.bits_remaining,
        shift_clock: s.serial_clock,
    }
}

pub fn capture_mbc(gb: &GameBoy) -> MbcSnapshot {
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
            ram_bank: 0, // RAM bank is encoded in the mapped register
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
            ram_enabled: true, // HuC3 RAM access is controlled by mode, not a separate flag
            mode: 0,
        },
    }
}

// ── Restore ──────────────────────────────────────────────────────

/// All the parsed snapshot data needed to construct a GameBoy.
pub struct Snapshot {
    pub cpu: CpuSnapshot,
    pub ppu: PpuSnapshot,
    pub apu: ApuSnapshot,
    pub timer: TimerSnapshot,
    pub dma: DmaSnapshot,
    pub serial: SerialSnapshot,
    pub mbc: MbcSnapshot,
    pub memory: Vec<MemoryRegion>,
}

impl GameBoy {
    /// Construct a GameBoy from a snapshot and the original cartridge.
    ///
    /// The cartridge provides the ROM; the snapshot provides everything
    /// else. The GameBoy is placed at an instruction boundary with the
    /// clock phase at Low (ready for the next rising edge).
    pub fn from_snapshot(cartridge: Cartridge, snap: Snapshot) -> GameBoy {
        use crate::cpu::mcycle::{BusDot, DotAction};
        use crate::memory::{ExternalBus, VramBus};
        use crate::ppu::{
            memory::{Oam, Vram},
            screen::Screen,
        };

        let find_region = |start: u16| -> Option<&[u8]> {
            snap.memory
                .iter()
                .find(|r| r.start == start)
                .map(|r| r.data.as_slice())
        };

        // Wave RAM for APU (extracted before building audio)
        let mut wave_ram = [0u8; 16];
        if let Some(data) = find_region(0xFF30) {
            let len = data.len().min(16);
            wave_ram[..len].copy_from_slice(&data[..len]);
        }

        // External bus (cartridge + WRAM)
        let mut external = ExternalBus::new(cartridge, None);
        if let Some(wram) = find_region(0xC000) {
            let len = wram.len().min(0x2000);
            external.work_ram[..len].copy_from_slice(&wram[..len]);
        }
        if let Some(cart_ram) = find_region(0xA000) {
            for (i, &byte) in cart_ram.iter().enumerate() {
                external.cartridge.write(0xA000 + i as u16, byte);
            }
        }

        // MBC bank state
        restore_mbc(&snap.mbc, external.cartridge.mbc_mut());

        let sgb = if external.cartridge.supports_sgb() {
            Some(crate::sgb::Sgb::new())
        } else {
            None
        };

        GameBoy {
            cpu: crate::cpu::Cpu::from_snapshot(&snap.cpu),
            screen: Screen::default(),
            ppu: crate::ppu::Ppu::from_snapshot(
                &snap.ppu,
                find_region(0xFE00).map(Oam::from_bytes).unwrap_or_default(),
            ),
            audio: Audio::from_snapshot(&snap.apu, wave_ram),
            timers: crate::timers::Timers::from_snapshot(&snap.timer),
            dma: crate::dma::Dma::from_snapshot(&snap.dma),
            serial: crate::serial_transfer::Registers::from_snapshot(&snap.serial),
            link: Box::new(crate::serial_transfer::Disconnected::new()),
            joypad: crate::joypad::Joypad::new(),
            interrupts: crate::interrupts::Registers {
                enabled: InterruptFlags::from_bits_retain(snap.cpu.ie),
                requested: InterruptFlags::from_bits_retain(snap.cpu.if_),
            },
            vram_bus: VramBus {
                vram: find_region(0x8000)
                    .map(Vram::from_bytes)
                    .unwrap_or_default(),
                latch: 0xFF,
            },
            high_ram: find_region(0xFF80)
                .map(crate::memory::HighRam::from_bytes)
                .unwrap_or_else(crate::memory::HighRam::new),
            external,
            sgb,
            last_read_value: 0,
            bus_trace: None,
            clock_phase: ClockPhase::Low,
            current_dot_action: DotAction::Idle,
            current_dot: BusDot::ZERO,
            staged_ppu_write: None,
        }
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
    }
}

// ── Capture ──────────────────────────────────────────────────────

pub fn capture_memory(gb: &GameBoy) -> Vec<u8> {
    let mut regions = Vec::new();

    // VRAM
    regions.push(MemoryRegion {
        start: 0x8000,
        data: gb.peek_range(0x8000, 0x2000),
    });

    // WRAM
    regions.push(MemoryRegion {
        start: 0xC000,
        data: gb.external_bus().work_ram.to_vec(),
    });

    // OAM
    regions.push(MemoryRegion {
        start: 0xFE00,
        data: gb.peek_range(0xFE00, 0x00A0),
    });

    // HRAM
    regions.push(MemoryRegion {
        start: 0xFF80,
        data: gb.high_ram().data().to_vec(),
    });

    // Wave RAM
    regions.push(MemoryRegion {
        start: 0xFF30,
        data: gb.audio().channels().ch3.ram.to_vec(),
    });

    // Cartridge RAM (full contents, not just the mapped bank)
    if let Some(ram) = gb.cartridge().ram() {
        if !ram.is_empty() {
            regions.push(MemoryRegion {
                start: 0xA000,
                data: ram,
            });
        }
    }

    build_memory_payload(&regions)
}
