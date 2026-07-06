//! Read-only inspection views of the console for the debugger panes.
//!
//! When paused, panes borrow the live console directly. When the core runs on
//! the emulation thread, the UI can't touch it, so each vblank the thread
//! copies the pane-relevant state into a [`ConsoleSnapshot`] and publishes it.
//! The panes render through the [`CpuSource`]/[`PpuSource`] traits (and the
//! core's `ReadInstructionMemory`), so one pane body serves both a live source
//! (`Cpu`, `Ppu`, `Console`) and its snapshot counterpart.

use missingno_gb::audio::{
    ApuSpec, Audio,
    channels::{Enabled, registers::VolumeAndEnvelope},
};
use missingno_gb::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::instructions::ReadInstructionMemory;
use missingno_gb::debugger::symbols::SymbolTable;
use missingno_gb::interrupts;
use missingno_gb::ppu::{
    Ppu, Register,
    memory::VramView,
    model::PpuModel,
    rendering::Mode,
    types::{
        control::Control,
        palette::Palette,
        sprites::{Sprite, SpriteId},
    },
};
use missingno_gb::{Console, Model};
use missingno_gbc::VramDmaStatus;
use std::sync::Arc;

use crate::app::console::{ConsoleColors, ConsoleUi};

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

/// A copied span of address space around PC, big enough for the instructions
/// pane's backward sweep and forward disassembly. Reads outside the span
/// return open-bus `0xFF` (the pane never sweeps past it).
pub struct MemoryWindow {
    base: u16,
    bytes: Vec<u8>,
}

impl MemoryWindow {
    /// Bytes captured before PC — covers `addresses_before`'s 128-byte sweep.
    const BEHIND: u16 = 128;
    /// Total span; the remainder ahead of PC covers the forward disassembly.
    const LEN: u16 = 512;

    fn capture<M: Model>(console: &Console<M>, pc: u16) -> Self {
        let base = pc.wrapping_sub(Self::BEHIND);
        let bytes = (0..Self::LEN)
            .map(|i| console.read(base.wrapping_add(i)))
            .collect();
        Self { base, bytes }
    }
}

impl ReadInstructionMemory for MemoryWindow {
    fn read(&self, address: u16) -> u8 {
        let offset = address.wrapping_sub(self.base) as usize;
        self.bytes.get(offset).copied().unwrap_or(0xFF)
    }
}

// --- Colours -----------------------------------------------------------------

/// The data needed to rebuild [`ConsoleColors`] UI-side while running, so the
/// user's palette choice (which can change mid-run) still applies on DMG.
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

impl ColorSnapshot {
    pub fn to_colors(&self, user_palette: &Palette) -> ConsoleColors {
        match self {
            Self::Dmg { sgb } => ConsoleColors::Dmg {
                palette: if *sgb {
                    Palette::CLASSIC
                } else {
                    *user_palette
                },
            },
            Self::Cgb {
                background,
                objects,
            } => ConsoleColors::Cgb {
                background: *background,
                objects: *objects,
            },
        }
    }
}

// --- CGB ---------------------------------------------------------------------

/// The CGB-only register state the sidebar draws — absent on DMG. Plain data,
/// read live when paused or copied into the snapshot while the core runs.
#[derive(Clone)]
pub struct CgbView {
    /// KEY1 speed bit: running at double speed.
    pub double_speed: bool,
    /// VBK ($FF4F) bank select.
    pub vram_bank: u8,
    /// Effective SVBK ($FF70) work-RAM bank.
    pub wram_bank: u8,
    /// OPRI ($FF6C) object-priority register.
    pub opri: u8,
    /// BCPS ($FF68) background palette index.
    pub bcps: u8,
    /// OCPS ($FF6A) object palette index.
    pub ocps: u8,
    /// VRAM-DMA (HDMA/GDMA) engine state.
    pub vram_dma: VramDmaStatus,
}

// --- Inspection source -------------------------------------------------------

/// Everything the debugger panes and sidebar render from, behind one
/// model-erased surface: the live console while paused, or the per-vblank
/// [`ConsoleSnapshot`] while the core runs on the emulation thread.
pub trait InspectSource {
    fn cpu(&self) -> &dyn CpuSource;
    fn ppu(&self) -> &dyn PpuSource;
    fn vram(&self) -> &dyn VramView;
    fn audio(&self) -> AudioView;
    fn interrupts(&self) -> interrupts::Registers;
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory;
    fn colors(&self, user_palette: &Palette) -> ConsoleColors;
    /// CGB register state for the sidebar; `None` on DMG.
    fn cgb(&self) -> Option<CgbView>;
    /// The 16KB ROM bank mapped at 0x4000–0x7FFF, for symbol resolution.
    fn switchable_rom_bank(&self) -> Option<u16>;
}

/// An owned [`InspectSource`] that can cross from the emulation thread.
/// A system's inspection surface, family-erased at the seam. Each family
/// exposes its typed surface through an accessor; a consumer asks for the
/// family it understands and falls back gracefully otherwise. The blanket
/// concrete impls hand each Game Boy source through as-is.
pub trait Inspection {
    fn as_gb(&self) -> Option<&dyn InspectSource> {
        None
    }
    #[cfg(feature = "vcs")]
    fn as_vcs(&self) -> Option<&crate::app::debugger::vcs::VcsInspectState> {
        None
    }
    #[cfg(feature = "sms")]
    fn as_sms(&self) -> Option<&crate::app::debugger::sms::SmsInspectState> {
        None
    }
    #[cfg(feature = "nes")]
    fn as_nes(&self) -> Option<&crate::app::debugger::nes::NesInspectState> {
        None
    }
}

impl<M: ConsoleUi> Inspection for Console<M>
where
    Console<M>: InspectSource,
{
    fn as_gb(&self) -> Option<&dyn InspectSource> {
        Some(self)
    }
}

impl Inspection for ConsoleSnapshot {
    fn as_gb(&self) -> Option<&dyn InspectSource> {
        Some(self)
    }
}

pub trait InspectSnapshot: Inspection + Send {
    fn frame(&self) -> u64;
    fn symbols(&self) -> &SymbolTable;
    fn cdl(&self) -> &CdlWindow;
}

/// The model-erased snapshot handed from the emulation thread to the UI.
pub type DebugView = Box<dyn InspectSnapshot>;

impl<M: ConsoleUi> InspectSource for Console<M> {
    fn cpu(&self) -> &dyn CpuSource {
        Console::cpu(self)
    }
    fn ppu(&self) -> &dyn PpuSource {
        Console::ppu(self)
    }
    fn vram(&self) -> &dyn VramView {
        Console::vram(self)
    }
    fn audio(&self) -> AudioView {
        AudioView::capture(Console::audio(self))
    }
    fn interrupts(&self) -> interrupts::Registers {
        Console::interrupts(self).clone()
    }
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory {
        self
    }
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        M::colors(self, user_palette)
    }
    fn cgb(&self) -> Option<CgbView> {
        M::cgb_view(self)
    }
    fn switchable_rom_bank(&self) -> Option<u16> {
        self.cartridge().switchable_rom_bank()
    }
}

// --- Console snapshot --------------------------------------------------------

/// A per-vblank copy of everything the debugger panes render, taken on the
/// emulation thread while the core runs there.
pub struct ConsoleSnapshot {
    pub cpu: CpuView,
    pub ppu: PpuView,
    pub vram: Box<dyn VramView + Send>,
    pub audio: AudioView,
    pub interrupts: interrupts::Registers,
    pub colors: ColorSnapshot,
    pub cgb: Option<CgbView>,
    pub switchable_rom_bank: Option<u16>,
    pub memory: MemoryWindow,
    pub symbols: Arc<SymbolTable>,
    pub cdl: CdlWindow,
    pub frame: u64,
}

impl ConsoleSnapshot {
    pub fn capture<M: ConsoleUi>(
        console: &Console<M>,
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
            colors: M::color_snapshot(console),
            cgb: M::cgb_view(console),
            switchable_rom_bank: console.cartridge().switchable_rom_bank(),
            memory: MemoryWindow::capture(console, console.cpu().ir_address),
            symbols,
            cdl,
            frame,
        }
    }
}

impl InspectSource for ConsoleSnapshot {
    fn cpu(&self) -> &dyn CpuSource {
        &self.cpu
    }
    fn ppu(&self) -> &dyn PpuSource {
        &self.ppu
    }
    fn vram(&self) -> &dyn VramView {
        &*self.vram
    }
    fn audio(&self) -> AudioView {
        self.audio.clone()
    }
    fn interrupts(&self) -> interrupts::Registers {
        self.interrupts.clone()
    }
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory {
        &self.memory
    }
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        self.colors.to_colors(user_palette)
    }
    fn cgb(&self) -> Option<CgbView> {
        self.cgb.clone()
    }
    fn switchable_rom_bank(&self) -> Option<u16> {
        self.switchable_rom_bank
    }
}

impl InspectSnapshot for ConsoleSnapshot {
    fn frame(&self) -> u64 {
        self.frame
    }
    fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }
    fn cdl(&self) -> &CdlWindow {
        &self.cdl
    }
}
