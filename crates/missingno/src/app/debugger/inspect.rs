//! Read-only inspection views of the console for the debugger panes.
//!
//! When paused, panes borrow the live console directly. When the core runs on
//! the emulation thread, the UI can't touch it, so each vblank the thread
//! copies the pane-relevant state into a [`ConsoleSnapshot`] and publishes it.
//! The panes render through the [`CpuSource`]/[`PpuSource`] traits (and the
//! core's `ReadInstructionMemory`), so one pane body serves both a live source
//! (`Cpu`, `Ppu`, `Console`) and its snapshot counterpart.

use missingno_gb::audio::Audio;
use missingno_gb::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use missingno_gb::debugger::instructions::ReadInstructionMemory;
use missingno_gb::interrupts;
use missingno_gb::ppu::{
    Ppu, Register,
    model::PpuModel,
    rendering::Mode,
    types::{
        control::Control,
        palette::Palette,
        sprites::{Sprite, SpriteId},
    },
};
use missingno_gb::{Console, Model};

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

// --- Console snapshot --------------------------------------------------------

/// A per-vblank copy of everything the debugger panes render, taken on the
/// emulation thread while the core runs there.
pub struct ConsoleSnapshot<M: ConsoleUi> {
    pub cpu: CpuView,
    pub ppu: PpuView,
    pub vram: <M::Ppu as PpuModel>::Vram,
    pub audio: Audio<M::Apu>,
    pub interrupts: interrupts::Registers,
    pub colors: ColorSnapshot,
    pub memory: MemoryWindow,
    pub frame: u64,
}

impl<M: ConsoleUi> ConsoleSnapshot<M>
where
    <M::Ppu as PpuModel>::Vram: Clone,
    M::Apu: Clone,
{
    pub fn capture(console: &Console<M>, frame: u64) -> Self {
        Self {
            cpu: CpuView::capture(console.cpu()),
            ppu: PpuView::capture(console.ppu()),
            vram: console.vram().clone(),
            audio: console.audio().clone(),
            interrupts: console.interrupts().clone(),
            colors: M::color_snapshot(console),
            memory: MemoryWindow::capture(console, console.cpu().ir_address),
            frame,
        }
    }
}

/// A model-erased snapshot handed from the emulation thread to the UI.
pub enum DebugView {
    Dmg(Box<ConsoleSnapshot<missingno_gb::Dmg>>),
    Cgb(Box<ConsoleSnapshot<missingno_gbc::Cgb>>),
}
