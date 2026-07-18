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
    inspect::FlagName { name: "z", bit: 7 },
    inspect::FlagName { name: "n", bit: 6 },
    inspect::FlagName { name: "h", bit: 5 },
    inspect::FlagName { name: "c", bit: 4 },
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
    };
    let hex16 = |name, value: u16| inspect::Register {
        name,
        value: value as u32,
        bits: 16,
        style: inspect::ValueStyle::Hex,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex8("a", Register8::A),
            inspect::Register {
                name: "f",
                value: cpu.flags().bits() as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(SM83_FLAGS),
            },
            hex8("b", Register8::B),
            hex8("c", Register8::C),
            hex8("d", Register8::D),
            hex8("e", Register8::E),
            hex8("h", Register8::H),
            hex8("l", Register8::L),
            hex16("sp", cpu.stack_pointer()),
            hex16("pc", cpu.ir_address()),
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
    pub memory: MemoryWindow,
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
            memory: MemoryWindow::capture(console, console.cpu().ir_address),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::debugger::Debugger;

    #[test]
    fn snapshot_register_groups_match_live() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger =
            Debugger::new(Console::<crate::Dmg>::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
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
}
