//! Read-only inspection views of the console for the debugger panes.
//!
//! The UI cannot touch the core while it runs on the emulation thread, so the
//! seam copies the pane-relevant state into a [`GbSnapshot`] and the panes
//! render from that. The section builders read the [`CpuSource`]/[`PpuSource`]
//! traits, so one body serves a capture and the live console the tests hold it
//! against.

use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::inspect;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{InspectSnapshot, RunningStatus};

use crate::audio::{
    ApuSpec, Audio,
    channels::{Enabled, registers::VolumeAndEnvelope},
};
use crate::cartridge::CartridgeView;
use crate::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use crate::interrupts;
use crate::ppu::{
    BgFifoCell, ObjFifoCell, Ppu, Register,
    model::PpuModel,
    rendering::Mode,
    types::{
        control::Control,
        palette::{Palette, PaletteIndex, PaletteMap},
        sprites::{Sprite, SpriteId, SpriteSize},
        tiles::TileAddressMode,
    },
};
use crate::{Console, Model};

/// The 40 hardware sprites.
const SPRITE_COUNT: usize = 40;

// --- CPU ---------------------------------------------------------------------

/// The CPU register state the sidebar draws — live [`Cpu`] or a snapshot copy.
pub trait CpuSource {
    fn get_register8(&self, register: Register8) -> u8;
    fn get_register16(&self, register: Register16) -> u16;
    fn flags(&self) -> Flags;
    fn ir_address(&self) -> u16;
    fn stack_pointer(&self) -> u16;
    fn halted(&self) -> bool;
    fn interrupts_enabled(&self) -> bool;
}

impl CpuSource for Cpu {
    fn get_register8(&self, register: Register8) -> u8 {
        Cpu::get_register8(self, register)
    }
    fn get_register16(&self, register: Register16) -> u16 {
        Cpu::get_register16(self, register)
    }
    fn flags(&self) -> Flags {
        self.flags
    }
    fn ir_address(&self) -> u16 {
        self.ir_address
    }
    fn stack_pointer(&self) -> u16 {
        self.stack_pointer
    }
    fn halted(&self) -> bool {
        self.halt.state == HaltState::Halted
    }
    fn interrupts_enabled(&self) -> bool {
        Cpu::interrupts_enabled(self)
    }
}

#[derive(Clone)]
pub struct CpuView {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    flags: Flags,
    stack_pointer: u16,
    ir_address: u16,
    halted: bool,
    ime: bool,
}

impl CpuView {
    fn capture(cpu: &Cpu) -> Self {
        Self {
            a: cpu.a,
            b: cpu.b,
            c: cpu.c,
            d: cpu.d,
            e: cpu.e,
            h: cpu.h,
            l: cpu.l,
            flags: cpu.flags,
            stack_pointer: cpu.stack_pointer,
            ir_address: cpu.ir_address,
            halted: cpu.halt.state == HaltState::Halted,
            ime: cpu.interrupts_enabled(),
        }
    }
}

impl CpuSource for CpuView {
    fn get_register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }
    fn get_register16(&self, register: Register16) -> u16 {
        match register {
            Register16::Bc => u16::from_be_bytes([self.b, self.c]),
            Register16::De => u16::from_be_bytes([self.d, self.e]),
            Register16::Hl => u16::from_be_bytes([self.h, self.l]),
            Register16::StackPointer => self.stack_pointer,
            Register16::Af => u16::from_be_bytes([self.a, self.flags.bits()]),
        }
    }
    fn flags(&self) -> Flags {
        self.flags
    }
    fn ir_address(&self) -> u16 {
        self.ir_address
    }
    fn stack_pointer(&self) -> u16 {
        self.stack_pointer
    }
    fn halted(&self) -> bool {
        self.halted
    }
    fn interrupts_enabled(&self) -> bool {
        self.ime
    }
}

/// Named bits of the SM83 flags register `f`.
const SM83_FLAGS: &[inspect::FlagName] = &[
    inspect::FlagName {
        name: "z",
        bit: 7,
        help: Some("zero flag — set when a result is zero"),
    },
    inspect::FlagName {
        name: "n",
        bit: 6,
        help: Some("subtract flag — set by a subtraction (used by DAA)"),
    },
    inspect::FlagName {
        name: "h",
        bit: 5,
        help: Some("half-carry flag — carry out of bit 3 (used by DAA)"),
    },
    inspect::FlagName {
        name: "c",
        bit: 4,
        help: Some("carry flag — set on carry or borrow"),
    },
];

/// The SM83 register file as one inspection group. Shared by the live debugger
/// (over the console's CPU) and the running snapshot (over its captured view),
/// so both produce identical groups. `pc` follows the debugger's convention of
/// the current instruction's fetch address.
pub fn cpu_register_groups(cpu: &impl CpuSource) -> Vec<inspect::RegisterGroup> {
    use inspect::RegisterPurpose::{PairHigh, PairLow, ProgramCounter, StackPointer};

    let hex8 = |name, register| inspect::Register {
        name,
        value: cpu.get_register8(register) as u32,
        bits: 8,
        style: inspect::ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    let hex16 = |name, value: u16| inspect::Register {
        name,
        value: value as u32,
        bits: 16,
        style: inspect::ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex8("a", Register8::A)
                .help("accumulator")
                .purpose(PairHigh("af")),
            inspect::Register {
                name: "f",
                value: cpu.flags().bits() as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(SM83_FLAGS),
                help: Some("flags register"),
                purpose: Some(PairLow("af")),
                active: None,
            },
            hex8("b", Register8::B)
                .help("general register B (high byte of BC)")
                .purpose(PairHigh("bc")),
            hex8("c", Register8::C)
                .help("general register C (low byte of BC)")
                .purpose(PairLow("bc")),
            hex8("d", Register8::D)
                .help("general register D (high byte of DE)")
                .purpose(PairHigh("de")),
            hex8("e", Register8::E)
                .help("general register E (low byte of DE)")
                .purpose(PairLow("de")),
            hex8("h", Register8::H)
                .help("general register H (high byte of HL)")
                .purpose(PairHigh("hl")),
            hex8("l", Register8::L)
                .help("general register L (low byte of HL)")
                .purpose(PairLow("hl")),
            hex16("sp", cpu.stack_pointer())
                .help("stack pointer")
                .purpose(StackPointer),
            hex16("pc", cpu.ir_address())
                .help("program counter")
                .purpose(ProgramCounter)
                .active(!cpu.halted()),
        ],
    }]
}

// --- PPU ---------------------------------------------------------------------

/// The PPU register/OAM state the tile-map, sprite, and PPU-sidebar panes draw.
pub trait PpuSource {
    fn control(&self) -> Control;
    fn mode(&self) -> Mode;
    /// The raw STAT byte — its mode bits plus the LYC-coincidence flag and the
    /// mode/LYC interrupt-enable bits the decoded rows don't otherwise carry.
    fn stat(&self) -> u8;
    fn ly(&self) -> u8;
    fn lx(&self) -> u8;
    /// The LY-compare register (LYC) driving the STAT coincidence flag.
    fn lyc(&self) -> u8;
    fn scx(&self) -> u8;
    fn scy(&self) -> u8;
    fn wx(&self) -> u8;
    fn wy(&self) -> u8;
    fn bgp(&self) -> u8;
    fn obp0(&self) -> u8;
    fn obp1(&self) -> u8;
    fn sprite(&self, id: SpriteId) -> Sprite;
    /// The background pixel-shifter's 8 stages, or `None` with the LCD off.
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]>;
    /// The object FIFO's 8 stages, or `None` with the LCD off.
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]>;
    /// The OAM-scan progress entry during mode 2, or `None` outside it (or with
    /// the LCD off).
    fn scan_counter(&self) -> Option<u8>;
}

impl<P: PpuModel> PpuSource for Ppu<P> {
    fn control(&self) -> Control {
        Ppu::control(self)
    }
    fn mode(&self) -> Mode {
        Ppu::mode(self)
    }
    fn stat(&self) -> u8 {
        self.read_register(Register::Status)
    }
    fn ly(&self) -> u8 {
        self.video.ly()
    }
    fn lx(&self) -> u8 {
        Ppu::lx(self)
    }
    fn lyc(&self) -> u8 {
        self.read_register(Register::InterruptOnScanline)
    }
    fn scx(&self) -> u8 {
        self.read_register(Register::BackgroundViewportX)
    }
    fn scy(&self) -> u8 {
        self.read_register(Register::BackgroundViewportY)
    }
    fn wx(&self) -> u8 {
        self.read_register(Register::WindowX)
    }
    fn wy(&self) -> u8 {
        self.read_register(Register::WindowY)
    }
    fn bgp(&self) -> u8 {
        self.palettes().background.output()
    }
    fn obp0(&self) -> u8 {
        self.palettes().sprite0.output()
    }
    fn obp1(&self) -> u8 {
        self.palettes().sprite1.output()
    }
    fn sprite(&self, id: SpriteId) -> Sprite {
        *Ppu::sprite(self, id)
    }
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]> {
        Ppu::bg_fifo(self)
    }
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]> {
        Ppu::obj_fifo(self)
    }
    fn scan_counter(&self) -> Option<u8> {
        Ppu::scan_counter(self)
    }
}

#[derive(Clone)]
pub struct PpuView {
    control: Control,
    mode: Mode,
    stat: u8,
    ly: u8,
    lx: u8,
    lyc: u8,
    scx: u8,
    scy: u8,
    wx: u8,
    wy: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    sprites: [Sprite; SPRITE_COUNT],
    bg_fifo: Option<[BgFifoCell; 8]>,
    obj_fifo: Option<[ObjFifoCell; 8]>,
    scan_counter: Option<u8>,
}

impl PpuView {
    fn capture<P: PpuModel>(ppu: &Ppu<P>) -> Self {
        Self {
            control: ppu.control(),
            mode: ppu.mode(),
            stat: ppu.read_register(Register::Status),
            ly: ppu.video.ly(),
            lx: ppu.lx(),
            lyc: ppu.read_register(Register::InterruptOnScanline),
            scx: ppu.read_register(Register::BackgroundViewportX),
            scy: ppu.read_register(Register::BackgroundViewportY),
            wx: ppu.read_register(Register::WindowX),
            wy: ppu.read_register(Register::WindowY),
            bgp: ppu.palettes().background.output(),
            obp0: ppu.palettes().sprite0.output(),
            obp1: ppu.palettes().sprite1.output(),
            sprites: std::array::from_fn(|i| *ppu.sprite(SpriteId(i as u8))),
            bg_fifo: ppu.bg_fifo(),
            obj_fifo: ppu.obj_fifo(),
            scan_counter: ppu.scan_counter(),
        }
    }
}

impl PpuSource for PpuView {
    fn control(&self) -> Control {
        self.control
    }
    fn mode(&self) -> Mode {
        self.mode
    }
    fn stat(&self) -> u8 {
        self.stat
    }
    fn ly(&self) -> u8 {
        self.ly
    }
    fn lx(&self) -> u8 {
        self.lx
    }
    fn lyc(&self) -> u8 {
        self.lyc
    }
    fn scx(&self) -> u8 {
        self.scx
    }
    fn scy(&self) -> u8 {
        self.scy
    }
    fn wx(&self) -> u8 {
        self.wx
    }
    fn wy(&self) -> u8 {
        self.wy
    }
    fn bgp(&self) -> u8 {
        self.bgp
    }
    fn obp0(&self) -> u8 {
        self.obp0
    }
    fn obp1(&self) -> u8 {
        self.obp1
    }
    fn sprite(&self, id: SpriteId) -> Sprite {
        self.sprites[id.0 as usize]
    }
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]> {
        self.bg_fifo
    }
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]> {
        self.obj_fifo
    }
    fn scan_counter(&self) -> Option<u8> {
        self.scan_counter
    }
}

// --- Audio -------------------------------------------------------------------

/// The APU state the audio pane and the sidebar's APU section draw, captured as
/// plain data so both serve a live [`Audio`] and its snapshot copy. Each view
/// carries the channel's register bytes plus the honest runtime summaries the
/// core already tracks (envelope volume, length counter, wave position).
#[derive(Clone)]
pub struct AudioView {
    /// NR52 power bit.
    pub enabled: bool,
    pub volume_left: u8,
    pub volume_right: u8,
    /// NR50 master volume / VIN byte.
    pub nr50: u8,
    /// Frame-sequencer step (0-7): the DIV-APU divider's phase.
    pub frame_sequencer_step: u8,
    /// The DIV bit the frame sequencer last sampled — its falling edge clocks
    /// the sequencer.
    pub prev_div_apu_bit: bool,
    pub ch1: PulseChannelView,
    pub ch2: PulseChannelView,
    pub ch3: WaveChannelView,
    pub ch4: NoiseChannelView,
}

/// A pulse channel (CH1 with its sweep, CH2 without).
#[derive(Clone, Copy)]
pub struct PulseChannelView {
    pub enabled: Enabled,
    /// NR10 sweep byte — present on CH1 only.
    pub sweep: Option<u8>,
    /// NR11/NR21 wave-duty and initial-length byte.
    pub duty_and_length: u8,
    /// NR12/NR22 volume-and-envelope byte.
    pub volume_and_envelope: VolumeAndEnvelope,
    /// 11-bit period (NR13 low, NR14 high three bits).
    pub period: u16,
    /// NRx4 length-enable bit.
    pub length_enabled: bool,
    pub length_counter: u16,
    /// Current envelope output volume (0-15).
    pub envelope_volume: u8,
    /// Envelope period counter — steps the volume when it reaches the pace.
    pub envelope_timer: u8,
    /// CH1 sweep shadow frequency, or `None` on the sweepless CH2.
    pub shadow_frequency: Option<u16>,
    /// CH1 sweep period counter, or `None` on CH2.
    pub sweep_timer: Option<u8>,
    /// CH1 sweep enabled, or `None` on CH2.
    pub sweep_enabled: Option<bool>,
    /// CH1 sweep has performed a negate calculation, or `None` on CH2.
    pub sweep_negate_used: Option<bool>,
}

#[derive(Clone, Copy)]
pub struct WaveChannelView {
    pub enabled: Enabled,
    /// NR30 DAC power bit.
    pub dac_enabled: bool,
    /// NR32 output-level byte.
    pub level: u8,
    /// Output volume as a fraction of full scale (the audio pane's readout).
    pub volume: f32,
    pub period: u16,
    pub length_enabled: bool,
    pub length_counter: u16,
    /// Wave-RAM sample position (0-31).
    pub wave_position: u8,
}

#[derive(Clone, Copy)]
pub struct NoiseChannelView {
    pub enabled: Enabled,
    /// NR42 volume-and-envelope byte.
    pub volume_and_envelope: VolumeAndEnvelope,
    /// NR43 clock-shift, LFSR width and divisor byte.
    pub frequency: u8,
    /// NR44 length-enable bit.
    pub length_enabled: bool,
    pub length_counter: u16,
    pub envelope_volume: u8,
    /// Envelope period counter — steps the volume when it reaches the pace.
    pub envelope_timer: u8,
    /// The noise LFSR (15-bit shift register).
    pub lfsr: u16,
}

impl AudioView {
    pub fn capture<A: ApuSpec>(audio: &Audio<A>) -> Self {
        let channels = audio.channels();
        let ch1 = &channels.ch1;
        let ch2 = &channels.ch2;
        let ch3 = &channels.ch3;
        let ch4 = &channels.ch4;
        Self {
            enabled: audio.enabled(),
            volume_left: audio.volume_left().0,
            volume_right: audio.volume_right().0,
            nr50: audio.nr50(),
            frame_sequencer_step: audio.frame_sequencer_step(),
            prev_div_apu_bit: audio.prev_div_apu_bit(),
            ch1: PulseChannelView {
                enabled: ch1.enabled,
                sweep: Some(ch1.sweep.0),
                duty_and_length: ch1.waveform_and_initial_length.0,
                volume_and_envelope: ch1.volume_and_envelope,
                period: ch1.period.0 & 0x7FF,
                length_enabled: ch1.length.enabled,
                length_counter: ch1.length.counter,
                envelope_volume: ch1.envelope.volume,
                envelope_timer: ch1.envelope.timer,
                shadow_frequency: Some(ch1.shadow_frequency),
                sweep_timer: Some(ch1.sweep_timer),
                sweep_enabled: Some(ch1.sweep_enabled),
                sweep_negate_used: Some(ch1.sweep_negate_used),
            },
            ch2: PulseChannelView {
                enabled: ch2.enabled,
                sweep: None,
                duty_and_length: ch2.waveform_and_initial_length.0,
                volume_and_envelope: ch2.volume_and_envelope,
                period: ch2.period.0 & 0x7FF,
                length_enabled: ch2.length.enabled,
                length_counter: ch2.length.counter,
                envelope_volume: ch2.envelope.volume,
                envelope_timer: ch2.envelope.timer,
                shadow_frequency: None,
                sweep_timer: None,
                sweep_enabled: None,
                sweep_negate_used: None,
            },
            ch3: WaveChannelView {
                enabled: ch3.enabled,
                dac_enabled: ch3.dac_enabled,
                level: ch3.volume.0,
                volume: ch3.volume.volume(),
                period: ch3.period.0 & 0x7FF,
                length_enabled: ch3.length.enabled,
                length_counter: ch3.length.counter,
                wave_position: ch3.wave_position,
            },
            ch4: NoiseChannelView {
                enabled: ch4.enabled,
                volume_and_envelope: ch4.volume_and_envelope,
                frequency: ch4.frequency_and_randomness.0,
                length_enabled: ch4.length.enabled,
                length_counter: ch4.length.counter,
                envelope_volume: ch4.envelope.volume,
                envelope_timer: ch4.envelope.timer,
                lfsr: ch4.lfsr,
            },
        }
    }
}

// --- Colours -----------------------------------------------------------------

/// The palette-independent colour data published while the core runs, so the
/// running panes can rebuild their render palettes with the live user palette
/// (which can change mid-run on DMG).
// One snapshot per vblank; boxing the CGB arrays would just add a hop.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum ColorSnapshot {
    Dmg {
        sgb: bool,
    },
    Cgb {
        background: [Palette; 8],
        objects: [Palette; 8],
    },
}

// --- Sidebar sections --------------------------------------------------------

/// The sidebar heading for a PPU mode.
pub fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::HorizontalBlank => "HBlank",
        Mode::VerticalBlank => "VBlank",
        Mode::OamScan => "OAM Scan",
        Mode::Drawing => "Drawing",
    }
}

/// The accent class for a PPU mode's inline detail.
fn mode_tone(mode: Mode) -> inspect::Tone {
    match mode {
        Mode::HorizontalBlank => inspect::Tone::Idle,
        Mode::VerticalBlank => inspect::Tone::Active,
        Mode::OamScan => inspect::Tone::Scanning,
        Mode::Drawing => inspect::Tone::Rendering,
    }
}

/// The five interrupt sources, in the order the interrupt table's columns show
/// them.
const INTERRUPT_SOURCES: [interrupts::Interrupt; 5] = [
    interrupts::Interrupt::VideoBetweenFrames,
    interrupts::Interrupt::VideoStatus,
    interrupts::Interrupt::Timer,
    interrupts::Interrupt::Serial,
    interrupts::Interrupt::Joypad,
];

// A system composes its own `Vec<Section>` from these shared part-builders
// over the CpuSource/PpuSource surfaces, deciding its own section summaries,
// activity, and where its console-specific content sits. DMG composes with
// `dmg_sidebar_sections`; CGB composes in `missingno-gbc` from the same parts
// plus its colour state.

/// The CPU section's collapsed summary.
pub fn cpu_summary(cpu: &impl CpuSource) -> String {
    inspect::register_file_summary(&cpu_register_groups(cpu))
}

/// The shared CPU block list: the register file's derived layout followed by
/// the interrupt table.
pub fn cpu_blocks(
    cpu: &impl CpuSource,
    ints: &interrupts::Registers,
) -> Vec<inspect::SectionBlock> {
    let mut blocks = inspect::register_file_blocks(cpu_register_groups(cpu));
    blocks.push(inspect::SectionBlock::Rule);
    blocks.push(inspect::SectionBlock::Table(interrupt_table(
        ints,
        cpu.interrupts_enabled(),
    )));
    blocks
}

fn interrupt_table(ints: &interrupts::Registers, ime: bool) -> inspect::BitTable {
    use inspect::Concept;
    inspect::BitTable {
        columns: vec![
            inspect::BitColumn::concept("VBlank", Concept::VBlank),
            inspect::BitColumn::concept("Stat", Concept::VideoStatus),
            inspect::BitColumn::concept("Timer", Concept::Timer),
            inspect::BitColumn::concept("Serial", Concept::Serial),
            inspect::BitColumn::concept("Joypad", Concept::Input),
        ],
        corner: Some(inspect::Flag {
            name: "IME",
            active: ime,
        }),
        rows: vec![
            inspect::BitRow {
                name: "IE",
                bits: INTERRUPT_SOURCES.iter().map(|&i| ints.enabled(i)).collect(),
                tone: inspect::Tone::Neutral,
            },
            inspect::BitRow {
                name: "IF",
                bits: INTERRUPT_SOURCES
                    .iter()
                    .map(|&i| ints.requested(i))
                    .collect(),
                tone: inspect::Tone::Pending,
            },
        ],
    }
}

/// The PPU section's collapsed summary.
pub fn ppu_summary(ppu: &impl PpuSource) -> String {
    format!("{} · ly {}", mode_label(ppu.mode()), ppu.ly())
}

/// The accented PPU-mode detail beside the section heading.
pub fn ppu_detail(ppu: &impl PpuSource) -> inspect::Detail {
    let mode = ppu.mode();
    inspect::Detail {
        text: mode_label(mode).to_string(),
        tone: mode_tone(mode),
    }
}

/// The ly/lx position sweeps: LY across the 154-line frame (144 visible lines
/// then 10 vblank lines), LX across the internal line counter (0 up to the SANU
/// line-end decode); mode boundaries within the line vary, so LX carries no
/// zones.
pub fn ppu_position_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    use inspect::{Sweep, SweepZone, Tone};

    let ly = Sweep::new("ly", ppu.ly() as u32, 154)
        .zones(vec![
            SweepZone {
                name: "visible",
                end: 144,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "vblank",
                end: 154,
                tone: Tone::Active,
            },
        ])
        .help("current scanline (LY) — 0..143 visible, 144..153 vblank");
    // The LX counter resets at the SANU line-end decode (value 113).
    let lx = Sweep::new("lx", ppu.lx() as u32, 114)
        .help("dot position within the scanline (LX counter)");

    inspect::SectionBlock::Sweeps(vec![ly, lx])
}

/// The raw STAT and LYC registers: STAT carries the LYC-coincidence flag and
/// the mode/LYC interrupt-enable bits the decoded rows don't otherwise show,
/// and LYC is the compare value that drives the coincidence flag.
pub fn ppu_status_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    // The scan counter advances only in mode 2; outside it the entry is stale,
    // so the row reads "-".
    let scan = match (ppu.mode(), ppu.scan_counter()) {
        (Mode::OamScan, Some(entry)) => entry.to_string(),
        _ => "-".to_string(),
    };
    inspect::SectionBlock::Rows(vec![
        inspect::Row::value("stat", format!("{:02X}", ppu.stat()))
            .help("LCD status (STAT) — mode, LYC coincidence, and mode/LYC interrupt enables"),
        inspect::Row::value("lyc", format!("{:02X}", ppu.lyc()))
            .help("LY compare (LYC) — matches LY to raise the STAT coincidence flag"),
        inspect::Row::value("scan", scan).help("OAM scan entry (mode 2)"),
    ])
}

/// The background enable/map/tile and scroll rows.
pub fn ppu_background_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("bg", control.background_and_window_enabled())
            .help("background & window enable (LCDC bit 0)"),
        inspect::Row::value("map", tile_map_addr(control.background_tile_map().0))
            .help("background tile-map base address"),
        inspect::Row::value("tile", tile_addr(control.tile_address_mode()))
            .help("tile-data addressing mode (LCDC bit 4)"),
        inspect::Row::value("scx", format!("{:02X}", ppu.scx())).help("background scroll X (SCX)"),
        inspect::Row::value("scy", format!("{:02X}", ppu.scy())).help("background scroll Y (SCY)"),
    ])
}

/// The window enable/map and position rows.
pub fn ppu_window_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("win", control.window_enabled()).help("window enable (LCDC bit 5)"),
        inspect::Row::value("map", tile_map_addr(control.window_tile_map().0))
            .help("window tile-map base address"),
        inspect::Row::value("wx", format!("{:02X}", ppu.wx())).help("window X position (WX)"),
        inspect::Row::value("wy", format!("{:02X}", ppu.wy())).help("window Y position (WY)"),
    ])
}

/// The sprite enable and size rows.
pub fn ppu_sprites_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("sprites", control.sprites_enabled())
            .help("object (sprite) enable (LCDC bit 1)"),
        inspect::Row::value(
            "size",
            match control.sprite_size() {
                SpriteSize::Single => "8×8",
                SpriteSize::Double => "8×16",
            },
        )
        .help("object size (LCDC bit 2)"),
    ])
}

/// The DMG background palette (BGP) as a packed shade-swatch row.
pub fn dmg_background_swatches(ppu: &impl PpuSource) -> inspect::SectionBlock {
    inspect::SectionBlock::Swatches(vec![inspect::SwatchRow::Shades {
        label: "bgp",
        packed: ppu.bgp(),
    }])
}

/// The two pixel FIFOs as DMG shade strips: each cell is the 2-bit colour
/// mapped through its palette register (BGP for background, the pixel's
/// OBP0/OBP1 select for objects) to a shade the frontend then resolves through
/// the user palette. A transparent object pixel (colour 0) and an off pipeline
/// render as unlit cells. Snapshots taken at vblank catch the FIFOs empty; the
/// strips fill when paused mid-scanline.
pub fn dmg_fifo_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    use inspect::PixelStrip;

    inspect::SectionBlock::Pixels(vec![
        PixelStrip::Shades {
            label: "bg fifo",
            cells: dmg_bg_strip(ppu.bg_fifo(), ppu.bgp()),
            help: Some("background pixel FIFO — colour through BGP; next pixel at left"),
        },
        PixelStrip::Shades {
            label: "obj fifo",
            cells: dmg_obj_strip(ppu.obj_fifo(), ppu.obp0(), ppu.obp1()),
            help: Some(
                "object pixel FIFO — colour through OBP0/OBP1; colour 0 transparent, discarded before palette",
            ),
        },
    ])
}

/// Each background cell's colour mapped through BGP to a shade — every cell is a
/// real colour (colour 0 is an opaque BG shade); an off pipeline is eight unlit
/// cells.
fn dmg_bg_strip(fifo: Option<[BgFifoCell; 8]>, bgp: u8) -> Vec<Option<u8>> {
    match fifo {
        Some(cells) => cells
            .iter()
            .map(|c| Some(PaletteMap(bgp).map(PaletteIndex(c.color)).0))
            .collect(),
        None => vec![None; 8],
    }
}

/// Each object cell's colour mapped through its OBP0/OBP1 select to a shade;
/// colour 0 (transparent) and an off pipeline render as empty cells.
fn dmg_obj_strip(fifo: Option<[ObjFifoCell; 8]>, obp0: u8, obp1: u8) -> Vec<Option<u8>> {
    match fifo {
        Some(cells) => cells
            .iter()
            .map(|c| {
                (c.color != 0).then(|| {
                    let obp = if c.palette == 0 { obp0 } else { obp1 };
                    PaletteMap(obp).map(PaletteIndex(c.color)).0
                })
            })
            .collect(),
        None => vec![None; 8],
    }
}

/// The DMG object palettes (OBP0/OBP1) as packed shade-swatch rows.
pub fn dmg_object_swatches(ppu: &impl PpuSource) -> inspect::SectionBlock {
    inspect::SectionBlock::Swatches(vec![
        inspect::SwatchRow::Shades {
            label: "obp0",
            packed: ppu.obp0(),
        },
        inspect::SwatchRow::Shades {
            label: "obp1",
            packed: ppu.obp1(),
        },
    ])
}

fn tile_map_addr(id: u8) -> &'static str {
    if id == 0 { "9800" } else { "9C00" }
}

fn tile_addr(mode: TileAddressMode) -> &'static str {
    match mode {
        TileAddressMode::Block0Block1 => "8000",
        TileAddressMode::Block2Block1 => "8800",
    }
}

// --- APU ----------------------------------------------------------------------

/// The NR14/NR24/NR34-style high byte reconstructed from the tracked period and
/// length-enable bit; the trigger bit is write-only and never held.
fn period_high_byte(period: u16, length_enabled: bool) -> u8 {
    (((period >> 8) & 0x07) as u8) | if length_enabled { 0x40 } else { 0x00 }
}

fn pulse_channel_block(
    label: &'static str,
    on_help: &'static str,
    ch: &PulseChannelView,
    nr1: &'static str,
    nr2: &'static str,
    nr3: &'static str,
    nr4: &'static str,
) -> inspect::SectionBlock {
    let mut rows = vec![inspect::Row::flag(label, ch.enabled.enabled).help(on_help)];
    if let Some(sweep) = ch.sweep {
        rows.push(
            inspect::Row::value("nr10", format!("{sweep:02X}"))
                .help("sweep pace / direction / step (NR10)"),
        );
    }
    rows.extend([
        inspect::Row::value(nr1, format!("{:02X}", ch.duty_and_length))
            .help("wave duty & initial length"),
        inspect::Row::value(nr2, format!("{:02X}", ch.volume_and_envelope.0))
            .help("initial volume & envelope"),
        inspect::Row::value(nr3, format!("{:02X}", ch.period & 0xFF)).help("period low byte"),
        inspect::Row::value(
            nr4,
            format!("{:02X}", period_high_byte(ch.period, ch.length_enabled)),
        )
        .help("period high & length-enable (trigger is write-only)"),
        inspect::Row::value("vol", ch.envelope_volume.to_string())
            .help("current envelope volume (0-15)"),
        inspect::Row::value("env timer", ch.envelope_timer.to_string())
            .help("envelope period counter — steps volume at the pace"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-64)"),
    ]);
    if let (Some(shadow), Some(timer), Some(enabled), Some(negate)) = (
        ch.shadow_frequency,
        ch.sweep_timer,
        ch.sweep_enabled,
        ch.sweep_negate_used,
    ) {
        rows.extend([
            inspect::Row::value("shadow", format!("{shadow:03X}"))
                .help("sweep shadow frequency (11-bit)"),
            inspect::Row::value("swp timer", timer.to_string())
                .help("sweep period counter — recalculates at the pace"),
            inspect::Row::flag("swp on", enabled).help("sweep unit enabled"),
            inspect::Row::flag("negate", negate)
                .help("a negate-direction sweep calculation has run"),
        ]);
    }
    inspect::SectionBlock::Rows(rows)
}

fn wave_channel_block(ch: &WaveChannelView) -> inspect::SectionBlock {
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("ch3", ch.enabled.enabled).help("channel 3 on (NR52 bit 2)"),
        inspect::Row::value("nr30", if ch.dac_enabled { "80" } else { "00" })
            .help("DAC power (NR30 bit 7)"),
        inspect::Row::value("nr32", format!("{:02X}", ch.level)).help("output level (NR32)"),
        inspect::Row::value("nr33", format!("{:02X}", ch.period & 0xFF)).help("period low byte"),
        inspect::Row::value(
            "nr34",
            format!("{:02X}", period_high_byte(ch.period, ch.length_enabled)),
        )
        .help("period high & length-enable (trigger is write-only)"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-256)"),
        inspect::Row::value("pos", ch.wave_position.to_string())
            .help("wave-RAM sample position (0-31)"),
    ])
}

fn noise_channel_block(ch: &NoiseChannelView) -> inspect::SectionBlock {
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("ch4", ch.enabled.enabled).help("channel 4 on (NR52 bit 3)"),
        inspect::Row::value("nr42", format!("{:02X}", ch.volume_and_envelope.0))
            .help("initial volume & envelope"),
        inspect::Row::value("nr43", format!("{:02X}", ch.frequency))
            .help("clock shift, LFSR width & divisor (NR43)"),
        inspect::Row::value("vol", ch.envelope_volume.to_string())
            .help("current envelope volume (0-15)"),
        inspect::Row::value("env timer", ch.envelope_timer.to_string())
            .help("envelope period counter — steps volume at the pace"),
        inspect::Row::value("lfsr", format!("{:04X}", ch.lfsr))
            .help("noise shift register (15-bit LFSR)"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-64)"),
    ])
}

/// NR51 sound-panning byte reconstructed from each channel's per-side output
/// enables: high nibble left (ch4..ch1), low nibble right (ch4..ch1).
fn panning_byte(audio: &AudioView) -> u8 {
    let sides = [
        audio.ch1.enabled,
        audio.ch2.enabled,
        audio.ch3.enabled,
        audio.ch4.enabled,
    ];
    let mut nr51 = 0u8;
    for (channel, enabled) in sides.iter().enumerate() {
        if enabled.output_right {
            nr51 |= 1 << channel;
        }
        if enabled.output_left {
            nr51 |= 1 << (channel + 4);
        }
    }
    nr51
}

/// The APU section, shared by DMG and CGB (the sound block is the same silicon):
/// the four channels' NRxx register bytes with the runtime summaries the core
/// tracks, plus the master NR50/NR51 registers. The header pip is the NR52 power
/// bit; the summary lists the powered-on channels.
pub fn apu_section(audio: &AudioView) -> inspect::Section {
    use inspect::SectionBlock::{Rows, Rule};

    let on: Vec<&str> = [
        (audio.ch1.enabled.enabled, "ch1"),
        (audio.ch2.enabled.enabled, "ch2"),
        (audio.ch3.enabled.enabled, "ch3"),
        (audio.ch4.enabled.enabled, "ch4"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    let summary = if !audio.enabled {
        "off".to_string()
    } else if on.is_empty() {
        "on".to_string()
    } else {
        on.join(" ")
    };

    inspect::Section {
        name: "APU",
        summary,
        active: Some(audio.enabled),
        detail: None,
        blocks: vec![
            Rows(vec![
                inspect::Row::value("nr50", format!("{:02X}", audio.nr50))
                    .help("master volume L/R & VIN (NR50)"),
                inspect::Row::value("nr51", format!("{:02X}", panning_byte(audio)))
                    .help("sound panning — per-channel L/R (NR51)"),
                inspect::Row::value("fs step", audio.frame_sequencer_step.to_string())
                    .help("frame-sequencer step (0-7) — DIV-APU divider phase"),
                inspect::Row::flag("div bit", audio.prev_div_apu_bit)
                    .help("DIV bit last sampled — its fall clocks the sequencer"),
            ]),
            Rule,
            pulse_channel_block(
                "ch1",
                "channel 1 on (NR52 bit 0)",
                &audio.ch1,
                "nr11",
                "nr12",
                "nr13",
                "nr14",
            ),
            Rule,
            pulse_channel_block(
                "ch2",
                "channel 2 on (NR52 bit 1)",
                &audio.ch2,
                "nr21",
                "nr22",
                "nr23",
                "nr24",
            ),
            Rule,
            wave_channel_block(&audio.ch3),
            Rule,
            noise_channel_block(&audio.ch4),
        ],
    }
}

/// The Cartridge section: the mapper, its current bank/enable state, and — on an
/// MBC3 with a clock — the RTC registers. Shared by the DMG and CGB sidebars,
/// and by the live console (paused) and running snapshot, so all agree.
pub fn cartridge_section(cart: &CartridgeView) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let rom_bank = cart
        .rom_bank
        .map_or_else(|| "—".to_owned(), |bank| bank.to_string());
    let summary = format!("{} · rom {}", cart.mapper, rom_bank);

    let mut rows = vec![
        Row::value("mapper", cart.mapper).help("cartridge memory bank controller"),
        Row::value("rom bank", rom_bank).help("16 KB ROM bank mapped at $4000"),
    ];
    if let Some(bank) = cart.ram_bank {
        rows.push(Row::value("ram bank", bank.to_string()).help("cart-RAM bank mapped at $A000"));
    }
    if let Some(enabled) = cart.ram_enabled {
        rows.push(
            Row::flag("ram enabled", enabled).help("cart-RAM/RTC access latch ($0000-$1FFF)"),
        );
    }
    if let Some(mode1) = cart.mode1 {
        let mode = if mode1 { "1 (advanced)" } else { "0 (simple)" };
        rows.push(Row::value("mode", mode).help("MBC1 banking mode ($6000-$7FFF)"));
    }

    let mut blocks = vec![SectionBlock::Rows(rows)];
    if let Some(rtc) = &cart.rtc {
        blocks.push(SectionBlock::Rule);
        blocks.push(SectionBlock::Rows(vec![
            Row::value("sec", rtc.seconds.to_string()).help("RTC seconds ($08)"),
            Row::value("min", rtc.minutes.to_string()).help("RTC minutes ($09)"),
            Row::value("hour", rtc.hours.to_string()).help("RTC hours ($0A)"),
            Row::value("day", rtc.day.to_string()).help("RTC day counter ($0B, $0C bit 0)"),
            Row::flag("halted", rtc.halted).help("RTC halt ($0C bit 6)"),
            Row::flag("latch armed", rtc.latch_ready).help("$6000 latch awaiting its 01 write"),
            Row::flag("day carry", rtc.day_carry).help("sticky day-counter overflow ($0C bit 7)"),
        ]));
    }

    inspect::Section {
        name: "Cartridge",
        summary,
        active: None,
        detail: None,
        blocks,
    }
}

// --- Timers -------------------------------------------------------------------

/// The timer registers plus the internal divider counter, captured so the live
/// console (paused) and the running snapshot serve the same section.
#[derive(Clone, Copy)]
pub struct TimersView {
    /// DIV ($FF04) — the divider's upper byte.
    pub div: u8,
    /// TIMA ($FF05) — the counter.
    pub tima: u8,
    /// TMA ($FF06) — the reload modulo.
    pub tma: u8,
    /// TAC ($FF07) — the control byte.
    pub tac: u8,
    /// The full 16-bit internal divider counter DIV reads its byte from.
    pub internal_counter: u16,
}

impl TimersView {
    pub fn capture(timers: &crate::timers::Timers) -> Self {
        use crate::timers::Register;
        Self {
            div: timers.read_register(Register::Divider),
            tima: timers.read_register(Register::Counter),
            tma: timers.read_register(Register::Modulo),
            tac: timers.read_register(Register::Control),
            internal_counter: timers.internal_counter(),
        }
    }
}

/// The Timers section: the DIV/TIMA/TMA/TAC registers, the TAC enable pip and
/// decoded increment frequency, and the internal 16-bit divider counter DIV is
/// a window onto. Shared by DMG and CGB (the same timer silicon).
pub fn timers_section(timers: &TimersView) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let enabled = timers.tac & 0b100 != 0;
    let frequency = match timers.tac & 0b11 {
        0b00 => 4096,
        0b01 => 262144,
        0b10 => 65536,
        _ => 16384,
    };

    inspect::Section {
        name: "Timers",
        summary: format!("div {:02X} · tima {:02X}", timers.div, timers.tima),
        active: Some(enabled),
        detail: None,
        blocks: vec![SectionBlock::Rows(vec![
            Row::value("div", format!("{:02X}", timers.div)).help("divider register (FF04)"),
            Row::value("tima", format!("{:02X}", timers.tima)).help("timer counter (FF05)"),
            Row::value("tma", format!("{:02X}", timers.tma)).help("timer modulo — reload (FF06)"),
            Row::value("tac", format!("{:02X}", timers.tac)).help("timer control (FF07)"),
            Row::flag("enabled", enabled).help("timer enable (TAC bit 2)"),
            Row::value("freq", format!("{frequency} Hz"))
                .help("TIMA increment frequency (TAC bits 0-1)"),
            Row::value("counter", format!("{:04X}", timers.internal_counter))
                .help("internal 16-bit divider counter"),
        ])],
    }
}

/// The DMG sidebar: CPU, PPU, Timers, APU and Cartridge sections composed from
/// the shared parts, with the DMG shade swatches sat with the registers they
/// describe. Shared by the live console (paused) and the running snapshot so
/// the two agree.
pub fn dmg_sidebar_sections(
    cpu: &impl CpuSource,
    ppu: &impl PpuSource,
    ints: &interrupts::Registers,
    timers: &TimersView,
    audio: &AudioView,
    cart: &CartridgeView,
) -> Vec<inspect::Section> {
    use inspect::SectionBlock::Rule;

    vec![
        inspect::Section {
            name: "CPU",
            summary: cpu_summary(cpu),
            active: Some(!cpu.halted()),
            detail: None,
            blocks: cpu_blocks(cpu, ints),
        },
        inspect::Section {
            name: "PPU",
            summary: ppu_summary(ppu),
            active: Some(ppu.control().video_enabled()),
            detail: Some(ppu_detail(ppu)),
            blocks: vec![
                ppu_position_block(ppu),
                ppu_status_block(ppu),
                Rule,
                ppu_background_block(ppu),
                dmg_background_swatches(ppu),
                Rule,
                ppu_window_block(ppu),
                Rule,
                ppu_sprites_block(ppu),
                dmg_object_swatches(ppu),
                Rule,
                dmg_fifo_block(ppu),
            ],
        },
        timers_section(timers),
        apu_section(audio),
        cartridge_section(cart),
    ]
}

// --- Console snapshot --------------------------------------------------------

/// A per-vblank copy of the model-shared debugger state, taken on the
/// emulation thread while the core runs there. The CGB build wraps this with
/// its extra register view.
#[derive(Clone)]
pub struct GbSnapshot {
    pub cpu: CpuView,
    pub ppu: PpuView,
    pub audio: AudioView,
    pub timers: TimersView,
    pub interrupts: interrupts::Registers,
    pub colors: ColorSnapshot,
    pub switchable_rom_bank: Option<u16>,
    pub cartridge: CartridgeView,
    pub symbols: Arc<SymbolTable>,
    pub cdl: CdlWindow,
    pub frame: u64,
}

impl GbSnapshot {
    pub fn capture<M: Model>(
        console: &Console<M>,
        colors: ColorSnapshot,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> Self {
        Self {
            cpu: CpuView::capture(console.cpu()),
            ppu: PpuView::capture(console.ppu()),
            audio: AudioView::capture(console.audio()),
            timers: TimersView::capture(console.timers()),
            interrupts: console.interrupts().clone(),
            colors,
            switchable_rom_bank: console.cartridge().switchable_rom_bank(),
            cartridge: console.cartridge().inspect(),
            symbols,
            cdl,
            frame,
        }
    }

    /// This capture stamped with the UI's frame counter.
    pub fn at_frame(&self, frame: u64) -> Self {
        Self {
            frame,
            ..self.clone()
        }
    }

    /// The one-line summary the frontend shows while the core runs.
    pub fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.cpu.ir_address.into(),
            sp: self.cpu.stack_pointer.into(),
            video_label: "PPU",
            video_summary: format!("{} · ly {}", mode_label(self.ppu.mode), self.ppu.ly),
            frame,
        }
    }
}

impl InspectSnapshot for GbSnapshot {
    fn frame(&self) -> u64 {
        self.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        self
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        cpu_register_groups(&self.cpu)
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        dmg_sidebar_sections(
            &self.cpu,
            &self.ppu,
            &self.interrupts,
            &self.timers,
            &self.audio,
            &self.cartridge,
        )
    }
    fn pc(&self) -> Option<u32> {
        Some(self.cpu.ir_address as u32)
    }
    fn symbols(&self) -> Option<&SymbolTable> {
        Some(&self.symbols)
    }
    fn cdl_window(&self) -> Option<&CdlWindow> {
        Some(&self.cdl)
    }
    fn bank_for(&self, address: u32) -> Option<u16> {
        match address {
            0x4000..=0x7FFF => self.switchable_rom_bank,
            _ => None,
        }
    }
    fn instruction_set(&self) -> Option<&dyn missingno_core::isa::InstructionSet> {
        Some(&crate::isa::Sm83)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::debugger::Debugger;

    fn stepped_dmg() -> Debugger<crate::Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger =
            Debugger::new(Console::<crate::Dmg>::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
        debugger
    }

    fn row_labels(section: &inspect::Section) -> Vec<String> {
        section
            .blocks
            .iter()
            .flat_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows.iter().map(|r| r.label.clone()).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn cartridge_section_shows_mbc3_rtc_rows() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x0f; // MBC3 + TIMER + BATTERY — carries an RTC
        let console = Console::<crate::Dmg>::new(Cartridge::new(rom, None), None);
        let section = cartridge_section(&console.cartridge().inspect());
        assert_eq!(section.name, "Cartridge");
        assert!(section.summary.starts_with("MBC3"), "{}", section.summary);
        let labels = row_labels(&section);
        for expected in ["mapper", "rom bank", "sec", "min", "hour", "day", "halted"] {
            assert!(
                labels.iter().any(|l| l == expected),
                "missing row {expected}"
            );
        }

        // A plain no-clock cart shows the section but no RTC rows.
        let plain = Console::<crate::Dmg>::new(Cartridge::new(vec![0u8; 0x8000], None), None);
        let plain_labels = row_labels(&cartridge_section(&plain.cartridge().inspect()));
        assert!(plain_labels.iter().all(|l| l != "sec"));
    }

    fn ran_console(capture: bool) -> Console<crate::Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut console = Console::<crate::Dmg>::new(Cartridge::new(rom, None), None);
        console.set_wave_capture(capture);
        // One frame's worth of steps fills the capture window.
        for _ in 0..20_000 {
            console.step();
        }
        console
    }

    #[test]
    fn capture_windows_fill_when_enabled() {
        let console = ran_console(true);
        let waves = console.channel_waves().expect("capture enabled");
        assert_eq!(waves.len(), 4);
        for wave in &waves {
            assert_eq!(wave.rate, 44100);
            assert!(!wave.levels.is_empty());
        }
    }

    #[test]
    fn no_capture_windows_when_disabled() {
        assert!(ran_console(false).channel_waves().is_none());
    }

    #[test]
    fn snapshot_register_groups_match_live() {
        let debugger = stepped_dmg();
        let live = debugger.register_groups();
        let snapshot = GbSnapshot::capture(
            debugger.game_boy(),
            ColorSnapshot::Dmg { sgb: false },
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.register_groups())
        );
    }

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let debugger = stepped_dmg();
        let console = debugger.game_boy();
        let audio = AudioView::capture(console.audio());
        let timers = TimersView::capture(console.timers());
        let cart = console.cartridge().inspect();
        let live = dmg_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &timers,
            &audio,
            &cart,
        );
        let snapshot = GbSnapshot::capture(
            console,
            ColorSnapshot::Dmg { sgb: false },
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.sidebar_sections())
        );
    }

    #[test]
    fn dmg_bg_strip_maps_colour_through_bgp() {
        // BGP 0b11_10_01_00: colour 0→0, 1→1, 2→2, 3→3 (identity).
        let cells = std::array::from_fn(|i| BgFifoCell {
            color: (i % 4) as u8,
            palette: 0,
        });
        let strip = dmg_bg_strip(Some(cells), 0b11_10_01_00);
        assert_eq!(strip[0], Some(0));
        assert_eq!(strip[1], Some(1));
        assert_eq!(strip[2], Some(2));
        assert_eq!(strip[3], Some(3));
        // An off pipeline is eight empty cells.
        assert_eq!(dmg_bg_strip(None, 0xE4), vec![None; 8]);
    }

    #[test]
    fn dmg_obj_strip_transparency_and_palette_select() {
        let cell = |color, palette| ObjFifoCell {
            color,
            palette,
            priority: 0,
        };
        // OBP0 identity; OBP1 = 0b00_01_10_11 maps colour 1→2, 3→0.
        let cells = [
            cell(0, 0), // transparent → empty
            cell(1, 0), // OBP0: shade 1
            cell(1, 1), // OBP1: shade 2
            cell(0, 1), // transparent → empty
            cell(2, 0), // OBP0: shade 2
            cell(3, 1), // OBP1: shade 0
            cell(0, 0),
            cell(0, 0),
        ];
        let strip = dmg_obj_strip(Some(cells), 0b11_10_01_00, 0b00_01_10_11);
        assert_eq!(strip[0], None);
        assert_eq!(strip[1], Some(1));
        assert_eq!(strip[2], Some(2));
        assert_eq!(strip[3], None);
        assert_eq!(strip[4], Some(2));
        assert_eq!(strip[5], Some(0));
        assert_eq!(dmg_obj_strip(None, 0xE4, 0xE4), vec![None; 8]);
    }

    #[test]
    fn apu_section_reports_power_and_channel_registers() {
        let audio = AudioView::capture(&Audio::<crate::audio::DmgApu>::post_boot(0));
        let section = apu_section(&audio);
        assert_eq!(section.name, "APU");
        // Post-boot: APU powered and CH1 running.
        assert_eq!(section.active, Some(true));
        assert!(section.summary.contains("ch1"));

        // CH1's NR12 register byte reads back the post-boot envelope value 0xF3.
        let nr12 = section
            .blocks
            .iter()
            .find_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows
                    .iter()
                    .find(|row| row.label == "nr12")
                    .map(|row| row.value.clone()),
                _ => None,
            })
            .expect("a CH1 NR12 row");
        assert_eq!(nr12, "F3");

        // The CH1 pip tracks the channel-on state.
        let ch1_on = section.blocks.iter().any(|block| match block {
            inspect::SectionBlock::Rows(rows) => rows
                .iter()
                .any(|row| row.label == "ch1" && row.active == Some(true)),
            _ => false,
        });
        assert!(ch1_on);
    }

    #[test]
    fn timers_section_carries_registers_and_divider_width() {
        let debugger = stepped_dmg();
        let timers = TimersView::capture(debugger.game_boy().timers());
        let section = timers_section(&timers);
        assert_eq!(section.name, "Timers");
        let labels = row_labels(&section);
        for expected in ["div", "tima", "tma", "tac", "enabled", "freq", "counter"] {
            assert!(
                labels.iter().any(|l| l == expected),
                "missing row {expected}"
            );
        }
        // The internal divider counter is the full 16-bit value — four hex digits.
        let counter = section
            .blocks
            .iter()
            .find_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows
                    .iter()
                    .find(|row| row.label == "counter")
                    .map(|row| row.value.clone()),
                _ => None,
            })
            .expect("a counter row");
        assert_eq!(counter.len(), 4, "divider counter is 16-bit hex: {counter}");
    }

    #[test]
    fn apu_section_carries_runtime_rows() {
        let audio = AudioView::capture(&Audio::<crate::audio::DmgApu>::post_boot(0));
        let section = apu_section(&audio);
        let labels = row_labels(&section);
        // Master block: frame-sequencer step and prev-DIV-bit pip.
        for expected in ["fs step", "div bit"] {
            assert!(labels.iter().any(|l| l == expected), "missing {expected}");
        }
        // CH1 carries the envelope timer plus its sweep runtime.
        for expected in ["env timer", "shadow", "swp timer", "swp on", "negate"] {
            assert!(labels.iter().any(|l| l == expected), "missing {expected}");
        }
        // CH4 carries the LFSR; CH2 (no sweep) carries no sweep rows.
        assert!(labels.iter().any(|l| l == "lfsr"));
    }

    #[test]
    fn ppu_status_block_carries_scan_row() {
        let debugger = stepped_dmg();
        let ppu = PpuView::capture(debugger.game_boy().ppu());
        let block = ppu_status_block(&ppu);
        let labels = match &block {
            inspect::SectionBlock::Rows(rows) => {
                rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>()
            }
            _ => panic!("expected rows"),
        };
        assert!(labels.iter().any(|l| l == "scan"), "missing scan row");
    }

    #[test]
    fn interrupt_table_tracks_enabled_and_requested() {
        use crate::interrupts::{Interrupt, InterruptFlags, Registers};

        let mut ints = Registers::new();
        ints.enabled = InterruptFlags::TIMER | InterruptFlags::JOYPAD;
        ints.request(Interrupt::VideoBetweenFrames);

        let table = interrupt_table(&ints, true);
        let names: Vec<_> = table.columns.iter().map(|column| column.name).collect();
        assert_eq!(names, ["VBlank", "Stat", "Timer", "Serial", "Joypad"]);
        let concepts: Vec<_> = table.columns.iter().map(|column| column.concept).collect();
        assert_eq!(
            concepts,
            [
                Some(inspect::Concept::VBlank),
                Some(inspect::Concept::VideoStatus),
                Some(inspect::Concept::Timer),
                Some(inspect::Concept::Serial),
                Some(inspect::Concept::Input),
            ]
        );
        assert_eq!(
            table.corner.map(|flag| (flag.name, flag.active)),
            Some(("IME", true))
        );
        assert_eq!(table.rows[0].name, "IE");
        assert_eq!(table.rows[0].bits, vec![false, false, true, false, true]);
        assert_eq!(table.rows[0].tone, inspect::Tone::Neutral);
        assert_eq!(table.rows[1].name, "IF");
        assert_eq!(table.rows[1].bits, vec![true, false, false, false, false]);
        assert_eq!(table.rows[1].tone, inspect::Tone::Pending);
    }

    #[test]
    fn dmg_swatch_blocks_carry_packed_registers() {
        let debugger = stepped_dmg();
        let ppu = PpuView::capture(debugger.game_boy().ppu());

        let rows: Vec<_> = [dmg_background_swatches(&ppu), dmg_object_swatches(&ppu)]
            .into_iter()
            .flat_map(|block| match block {
                inspect::SectionBlock::Swatches(rows) => rows,
                _ => panic!("expected swatches"),
            })
            .collect();
        let expected = [
            ("bgp", ppu.bgp()),
            ("obp0", ppu.obp0()),
            ("obp1", ppu.obp1()),
        ];
        assert_eq!(rows.len(), expected.len());
        for (row, (label, packed)) in rows.iter().zip(expected) {
            match row {
                inspect::SwatchRow::Shades {
                    label: got_label,
                    packed: got_packed,
                } => {
                    assert_eq!(*got_label, label);
                    assert_eq!(*got_packed, packed);
                }
                _ => panic!("expected packed shades"),
            }
        }

        // The DMG PPU section places both swatch blocks with its registers.
        let console = debugger.game_boy();
        let audio = AudioView::capture(console.audio());
        let timers = TimersView::capture(console.timers());
        let cart = console.cartridge().inspect();
        let sections = dmg_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &timers,
            &audio,
            &cart,
        );
        let swatch_blocks = sections[1]
            .blocks
            .iter()
            .filter(|block| matches!(block, inspect::SectionBlock::Swatches(_)))
            .count();
        assert_eq!(swatch_blocks, 2);
    }
}
