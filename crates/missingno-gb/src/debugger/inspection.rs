//! Read-only inspection views of the console for the debugger panes.
//!
//! When paused, panes borrow the live console directly. When the core runs on
//! the emulation thread, the UI can't touch it, so each vblank the thread
//! copies the pane-relevant state into a [`GbSnapshot`] and publishes it. The
//! panes render through the [`CpuSource`]/[`PpuSource`] traits (and
//! [`ReadInstructionMemory`]), so one pane body serves both a live source
//! (`Cpu`, `Ppu`, `Console`) and its snapshot counterpart.

use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::inspect;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::InspectSnapshot;

use crate::audio::{
    ApuSpec, Audio,
    channels::{Enabled, registers::VolumeAndEnvelope},
};
use crate::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use crate::debugger::instructions::ReadInstructionMemory;
use crate::interrupts;
use crate::ppu::{
    BgFifoCell, ObjFifoCell, Ppu, Register,
    memory::VramView,
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
    let hex8 = |name, register| inspect::Register {
        name,
        value: cpu.get_register8(register) as u32,
        bits: 8,
        style: inspect::ValueStyle::Hex,
        help: None,
    };
    let hex16 = |name, value: u16| inspect::Register {
        name,
        value: value as u32,
        bits: 16,
        style: inspect::ValueStyle::Hex,
        help: None,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex8("a", Register8::A).help("accumulator"),
            inspect::Register {
                name: "f",
                value: cpu.flags().bits() as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(SM83_FLAGS),
                help: Some("flags register"),
            },
            hex8("b", Register8::B).help("general register B (high byte of BC)"),
            hex8("c", Register8::C).help("general register C (low byte of BC)"),
            hex8("d", Register8::D).help("general register D (high byte of DE)"),
            hex8("e", Register8::E).help("general register E (low byte of DE)"),
            hex8("h", Register8::H).help("general register H (high byte of HL)"),
            hex8("l", Register8::L).help("general register L (low byte of HL)"),
            hex16("sp", cpu.stack_pointer()).help("stack pointer"),
            hex16("pc", cpu.ir_address()).help("program counter"),
        ],
    }]
}

// --- PPU ---------------------------------------------------------------------

/// The PPU register/OAM state the tile-map, sprite, and PPU-sidebar panes draw.
pub trait PpuSource {
    fn control(&self) -> Control;
    fn mode(&self) -> Mode;
    fn ly(&self) -> u8;
    fn lx(&self) -> u8;
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
}

impl<P: PpuModel> PpuSource for Ppu<P> {
    fn control(&self) -> Control {
        Ppu::control(self)
    }
    fn mode(&self) -> Mode {
        Ppu::mode(self)
    }
    fn ly(&self) -> u8 {
        self.video.ly()
    }
    fn lx(&self) -> u8 {
        Ppu::lx(self)
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
}

#[derive(Clone)]
pub struct PpuView {
    control: Control,
    mode: Mode,
    ly: u8,
    lx: u8,
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
}

impl PpuView {
    fn capture<P: PpuModel>(ppu: &Ppu<P>) -> Self {
        Self {
            control: ppu.control(),
            mode: ppu.mode(),
            ly: ppu.video.ly(),
            lx: ppu.lx(),
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
    fn ly(&self) -> u8 {
        self.ly
    }
    fn lx(&self) -> u8 {
        self.lx
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
}

// --- Audio -------------------------------------------------------------------

/// The APU state the audio pane draws, captured as plain data so the pane
/// serves both a live [`Audio`] and its snapshot copy.
#[derive(Clone)]
pub struct AudioView {
    pub enabled: bool,
    pub volume_left: u8,
    pub volume_right: u8,
    pub ch1: EnvelopeChannelView,
    pub ch2: EnvelopeChannelView,
    pub ch3: WaveChannelView,
    pub ch4: EnvelopeChannelView,
}

#[derive(Clone, Copy)]
pub struct EnvelopeChannelView {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
}

#[derive(Clone, Copy)]
pub struct WaveChannelView {
    pub enabled: Enabled,
    /// Output volume as a fraction of full scale.
    pub volume: f32,
}

impl AudioView {
    pub fn capture<A: ApuSpec>(audio: &Audio<A>) -> Self {
        let channels = audio.channels();
        Self {
            enabled: audio.enabled(),
            volume_left: audio.volume_left().0,
            volume_right: audio.volume_right().0,
            ch1: EnvelopeChannelView {
                enabled: channels.ch1.enabled,
                volume_and_envelope: channels.ch1.volume_and_envelope,
            },
            ch2: EnvelopeChannelView {
                enabled: channels.ch2.enabled,
                volume_and_envelope: channels.ch2.volume_and_envelope,
            },
            ch3: WaveChannelView {
                enabled: channels.ch3.enabled,
                volume: channels.ch3.volume.volume(),
            },
            ch4: EnvelopeChannelView {
                enabled: channels.ch4.enabled,
                volume_and_envelope: channels.ch4.volume_and_envelope,
            },
        }
    }
}

// --- Memory window -----------------------------------------------------------

/// Bytes captured before PC — covers `addresses_before`'s 128-byte sweep.
const WINDOW_BEHIND: u16 = 128;
/// Total span; the remainder ahead of PC covers the forward disassembly.
const WINDOW_LEN: u16 = 512;

/// A copied span of address space around PC, big enough for the instructions
/// pane's backward sweep and forward disassembly, captured with the CPU's
/// 16-bit address wrap.
fn capture_memory_window<M: Model>(console: &Console<M>, pc: u16) -> inspect::MemoryWindow {
    let base = pc.wrapping_sub(WINDOW_BEHIND);
    let bytes = (0..WINDOW_LEN)
        .map(|i| console.read(base.wrapping_add(i)))
        .collect();
    inspect::MemoryWindow {
        base: base as u32,
        bytes,
    }
}

/// Reads through the captured window with the CPU's 16-bit wrap; addresses
/// outside the span return open-bus `0xFF` (the pane never sweeps past it).
impl ReadInstructionMemory for inspect::MemoryWindow {
    fn read(&self, address: u16) -> u8 {
        let offset = address.wrapping_sub(self.base as u16) as usize;
        self.bytes.get(offset).copied().unwrap_or(0xFF)
    }
}

// --- Colours -----------------------------------------------------------------

/// The palette-independent colour data published while the core runs, so the
/// running panes can rebuild their render palettes with the live user palette
/// (which can change mid-run on DMG).
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
    format!(
        "pc {:04X} · sp {:04X}",
        cpu.ir_address(),
        cpu.stack_pointer()
    )
}

/// The shared CPU block list: the pc/sp pointers, the `af`/`bc`/`de`/`hl`
/// register pairs, and the interrupt table.
pub fn cpu_blocks(
    cpu: &impl CpuSource,
    ints: &interrupts::Registers,
) -> Vec<inspect::SectionBlock> {
    let hex8 = |name, register| inspect::Register {
        name,
        value: cpu.get_register8(register) as u32,
        bits: 8,
        style: inspect::ValueStyle::Hex,
        help: None,
    };
    let f = inspect::Register {
        name: "f",
        value: cpu.flags().bits() as u32,
        bits: 8,
        style: inspect::ValueStyle::Flags(SM83_FLAGS),
        help: Some("flags register"),
    };
    let pairs = vec![
        inspect::RegisterPair {
            high: hex8("a", Register8::A).help("accumulator"),
            low: f,
        },
        inspect::RegisterPair {
            high: hex8("b", Register8::B).help("general register B (high byte of BC)"),
            low: hex8("c", Register8::C).help("general register C (low byte of BC)"),
        },
        inspect::RegisterPair {
            high: hex8("d", Register8::D).help("general register D (high byte of DE)"),
            low: hex8("e", Register8::E).help("general register E (low byte of DE)"),
        },
        inspect::RegisterPair {
            high: hex8("h", Register8::H).help("general register H (high byte of HL)"),
            low: hex8("l", Register8::L).help("general register L (low byte of HL)"),
        },
    ];
    let pointer = |name, value: u16, active, help| inspect::Pointer {
        register: inspect::Register {
            name,
            value: value as u32,
            bits: 16,
            style: inspect::ValueStyle::Hex,
            help: Some(help),
        },
        active,
    };

    vec![
        inspect::SectionBlock::Pointers(vec![
            pointer(
                "pc",
                cpu.ir_address(),
                Some(!cpu.halted()),
                "program counter",
            ),
            pointer("sp", cpu.stack_pointer(), None, "stack pointer"),
        ]),
        inspect::SectionBlock::Rule,
        inspect::SectionBlock::Pairs(pairs),
        inspect::SectionBlock::Rule,
        inspect::SectionBlock::Table(interrupt_table(ints, cpu.interrupts_enabled())),
    ]
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

/// The DMG sidebar: CPU and PPU sections composed from the shared parts, with
/// the DMG shade swatches sat with the registers they describe. Shared by the
/// live console (paused) and the running snapshot so the two agree.
pub fn dmg_sidebar_sections(
    cpu: &impl CpuSource,
    ppu: &impl PpuSource,
    ints: &interrupts::Registers,
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
    ]
}

// --- Console snapshot --------------------------------------------------------

/// A per-vblank copy of the model-shared debugger state, taken on the
/// emulation thread while the core runs there. The CGB build wraps this with
/// its extra register view.
pub struct GbSnapshot {
    pub cpu: CpuView,
    pub ppu: PpuView,
    pub vram: Box<dyn VramView + Send>,
    pub audio: AudioView,
    pub interrupts: interrupts::Registers,
    pub colors: ColorSnapshot,
    pub switchable_rom_bank: Option<u16>,
    pub memory: inspect::MemoryWindow,
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
    ) -> Self
    where
        <M::Ppu as PpuModel>::Vram: Clone + Send + 'static,
    {
        Self {
            cpu: CpuView::capture(console.cpu()),
            ppu: PpuView::capture(console.ppu()),
            vram: Box::new(console.vram().clone()),
            audio: AudioView::capture(console.audio()),
            interrupts: console.interrupts().clone(),
            colors,
            switchable_rom_bank: console.cartridge().switchable_rom_bank(),
            memory: capture_memory_window(console, console.cpu().ir_address),
            symbols,
            cdl,
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
        dmg_sidebar_sections(&self.cpu, &self.ppu, &self.interrupts)
    }
    fn memory_window(&self) -> Option<&inspect::MemoryWindow> {
        Some(&self.memory)
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
        let live = dmg_sidebar_sections(console.cpu(), console.ppu(), console.interrupts());
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
        let sections = dmg_sidebar_sections(console.cpu(), console.ppu(), console.interrupts());
        let swatch_blocks = sections[1]
            .blocks
            .iter()
            .filter(|block| matches!(block, inspect::SectionBlock::Swatches(_)))
            .count();
        assert_eq!(swatch_blocks, 2);
    }
}
