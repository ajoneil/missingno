//! The NES family's debugger seam: its inspection state and the machine
//! binding over the console. One owned state struct serves both the paused
//! view (refreshed after every step) and the per-frame snapshot the running
//! view renders from.

use std::sync::Arc;
use std::time::Duration;

use missingno_core::TvStandard;
use missingno_core::inspect::{
    BitColumn, BitRow, BitTable, FlagName, Register, RegisterGroup, Row, Section, SectionBlock,
    Sweep, SweepZone, Tone, ValueStyle,
};
use missingno_core::machine::Machine;
use missingno_core::ports::{
    ControlDescriptor, PeripheralDescriptor, PeripheralId, PlugError, PortDescriptor, PortId,
    Provider,
};
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, ControlSite, DebugView, InspectSnapshot, RunningStatus,
};
use missingno_core::video::{DisplayTechnology, Frame as DisplayFrame, IndexedFrame};
use missingno_mos_6502::disasm;
use rgb::RGB8;

use crate::console::Nes;
use crate::ppu::{self, Frame};

/// CPU cycles per frame step, generous over the ~29.8k typical.
const FRAME_BUDGET: u32 = 200_000;

/// NTSC pixel aspect at the 2C02's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

const DISASSEMBLY_ROWS: usize = 12;
const JSR: u8 = 0x20;

/// Named bits of the 2A03's 6502 status register `p`; the B flag is not
/// architectural.
const MOS6502_FLAGS: &[FlagName] = &[
    FlagName {
        name: "n",
        bit: 7,
        help: Some("negative flag — bit 7 of the result"),
    },
    FlagName {
        name: "v",
        bit: 6,
        help: Some("overflow flag — signed overflow"),
    },
    FlagName {
        name: "d",
        bit: 3,
        help: Some("decimal-mode flag (ignored by the 2A03)"),
    },
    FlagName {
        name: "i",
        bit: 2,
        help: Some("interrupt-disable flag"),
    },
    FlagName {
        name: "z",
        bit: 1,
        help: Some("zero flag — set when a result is zero"),
    },
    FlagName {
        name: "c",
        bit: 0,
        help: Some("carry flag — set on carry or borrow"),
    },
];

#[derive(Clone, Default)]
pub struct NesInspectState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    pub scanline: u16,
    pub dot: u16,
    pub ppu_control: u8,
    pub ppu_mask: u8,
    pub ppu_status: u8,
    pub scroll_v: u16,
    pub disassembly: Vec<DisasmRow>,
    pub frame: u64,
}

#[derive(Clone)]
pub struct DisasmRow {
    pub address: u16,
    pub text: String,
    pub current: bool,
}

/// The per-frame snapshot for the running view.
pub struct NesSnapshot {
    pub state: NesInspectState,
}

impl NesSnapshot {
    pub fn new(state: NesInspectState) -> Self {
        NesSnapshot { state }
    }
}

impl InspectSnapshot for NesSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        cpu_register_groups(&self.state)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        nes_sidebar_sections(&self.state)
    }
}

/// The 2A03 register file as one inspection group, shared by the live view and
/// the running snapshot. The stack pointer shows as the page-1 address it
/// selects rather than as the raw `s` offset.
fn cpu_register_groups(state: &NesInspectState) -> Vec<RegisterGroup> {
    use missingno_core::inspect::RegisterPurpose;

    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", state.a as u32, 8).help("accumulator"),
            hex("x", state.x as u32, 8).help("X index register"),
            hex("y", state.y as u32, 8).help("Y index register"),
            hex("sp", 0x0100 | state.s as u32, 16)
                .help("stack pointer (offset into page 1)")
                .purpose(RegisterPurpose::StackPointer),
            Register {
                name: "p",
                value: state.p as u32,
                bits: 8,
                style: ValueStyle::Flags(MOS6502_FLAGS),
                help: Some("processor status flags"),
                purpose: None,
                active: None,
            },
            hex("pc", state.pc as u32, 16)
                .help("program counter")
                .purpose(RegisterPurpose::ProgramCounter),
        ],
    }]
}

/// PPUCTRL ($2000) bits, high to low.
const PPUCTRL_BITS: &[&str] = &["nmi", "slave", "spr16", "bg", "spr", "inc", "nt1", "nt0"];
/// PPUMASK ($2001) bits, high to low.
const PPUMASK_BITS: &[&str] = &["blue", "green", "red", "spr", "bg", "sprL", "bgL", "gray"];
/// PPUSTATUS ($2002) bits, high to low; the low five are open bus.
const PPUSTATUS_BITS: &[&str] = &["vbl", "s0", "ovf", "-", "-", "-", "-", "-"];

/// The NES sidebar sections, shared by the live view and the running snapshot:
/// the 2A03 register file plus the 2C02's position and control registers.
fn nes_sidebar_sections(state: &NesInspectState) -> Vec<Section> {
    let mut sections = missingno_core::inspect::default_sections(cpu_register_groups(state));
    sections.push(ppu_section(state));
    sections
}

fn ppu_section(state: &NesInspectState) -> Section {
    use crate::ppu::{DOTS_PER_LINE, LINES_PER_FRAME, PRERENDER_LINE, VBLANK_LINE, VISIBLE_LINES};

    // The 2C02 frame: visible lines, one post-render idle line, the vblank
    // lines, then the pre-render line. The dot cycle within a line has fine
    // structure that varies with rendering, so `dot` carries no zones.
    let scanline = Sweep::new("scanline", state.scanline as u32, LINES_PER_FRAME as u32)
        .zones(vec![
            SweepZone {
                name: "visible",
                end: VISIBLE_LINES as u32,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "post",
                end: VBLANK_LINE as u32,
                tone: Tone::Idle,
            },
            SweepZone {
                name: "vblank",
                end: PRERENDER_LINE as u32,
                tone: Tone::Active,
            },
            SweepZone {
                name: "pre-render",
                end: LINES_PER_FRAME as u32,
                tone: Tone::Scanning,
            },
        ])
        .help("PPU scanline — visible, post-render, vblank, then pre-render");
    let dot = Sweep::new("dot", state.dot as u32, DOTS_PER_LINE as u32)
        .help("PPU dot (cycle) within the scanline");

    Section {
        name: "PPU",
        summary: format!("scanline {} · dot {}", state.scanline, state.dot),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Sweeps(vec![scanline, dot]),
            SectionBlock::Rows(vec![
                Row::value("scroll", format!("{:04X}", state.scroll_v))
                    .help("PPU scroll/address (v)"),
            ]),
            SectionBlock::Rule,
            SectionBlock::Table(bit_table("2000", PPUCTRL_BITS, state.ppu_control)),
            SectionBlock::Table(bit_table("2001", PPUMASK_BITS, state.ppu_mask)),
            SectionBlock::Table(bit_table("2002", PPUSTATUS_BITS, state.ppu_status)),
        ],
    }
}

/// A one-row bit table decoding `value`'s bits, high to low, under `columns`.
fn bit_table(name: &'static str, columns: &'static [&'static str], value: u8) -> BitTable {
    BitTable {
        columns: columns.iter().map(|&name| BitColumn::plain(name)).collect(),
        corner: None,
        rows: vec![BitRow {
            name,
            bits: (0..8).rev().map(|bit| value & (1 << bit) != 0).collect(),
            tone: Tone::Neutral,
        }],
    }
}

/// iNES media carries the `NES\x1A` magic in its first four bytes.
pub fn is_nes_rom(rom: &[u8]) -> bool {
    rom.len() >= 4 && &rom[0..4] == b"NES\x1A"
}

pub struct NesSystem;

impl Machine for NesSystem {
    type Core = Nes;
    type Frame = Frame;
    type InspectState = NesInspectState;

    /// One NTSC frame: 262 lines × 341 dots ÷ 3 CPU cycles ≈ 29780 cycles.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_639);
    const RUN_BUDGET: u32 = 400_000;

    fn pc(nes: &Nes) -> u16 {
        nes.cpu.pc
    }

    fn peek(nes: &Nes, address: u16) -> u8 {
        nes.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn missingno_core::isa::InstructionSet> {
        Some(&missingno_mos_6502::isa::Mos6502)
    }

    fn step_instruction(nes: &mut Nes) {
        nes.step_instruction();
    }

    fn take_frame(nes: &mut Nes) -> Option<Frame> {
        nes.take_frame()
    }

    fn step_frame(nes: &mut Nes) -> Option<Frame> {
        nes.step_frame(FRAME_BUDGET)
    }

    fn power_cycle(nes: &mut Nes) {
        nes.power_cycle();
    }

    fn apply_control(nes: &mut Nes, control: ControlId, input: ControlInput) {
        apply_control(nes, control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(_nes: &Nes, port: PortId) -> Option<PeripheralId> {
        (port == CONTROLLER_PORT).then_some(CONTROLLER)
    }

    fn plug(_nes: &mut Nes, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        match (port, peripheral) {
            (CONTROLLER_PORT, CONTROLLER) => Ok(()),
            (CONTROLLER_PORT, _) => Err(PlugError::NotAccepted),
            _ => Err(PlugError::UnknownPort),
        }
    }

    fn drain_audio_samples(nes: &mut Nes) -> Vec<(f32, f32)> {
        nes.drain_audio_samples()
    }

    fn video_out(_nes: &Nes) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: TvStandard::Ntsc,
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn display_frame(frame: &Frame) -> DisplayFrame {
        DisplayFrame::Indexed(IndexedFrame {
            width: ppu::PIXELS_PER_LINE as u32,
            height: ppu::VISIBLE_LINES as u32,
            pixels: frame.pixels.clone().into(),
            palette: nes_palette(),
        })
    }

    fn blank_display() -> DisplayFrame {
        DisplayFrame::Indexed(IndexedFrame::blank(
            ppu::PIXELS_PER_LINE as u32,
            ppu::VISIBLE_LINES as u32,
            nes_palette(),
        ))
    }

    fn step_over_target(nes: &Nes) -> Option<u16> {
        (nes.peek(nes.cpu.pc) == JSR).then(|| nes.cpu.pc.wrapping_add(3))
    }

    fn inspect(nes: &Nes, frame_count: u64) -> NesInspectState {
        let cpu = &nes.cpu;
        let mut disassembly = Vec::with_capacity(DISASSEMBLY_ROWS);
        let mut address = cpu.pc;
        for i in 0..DISASSEMBLY_ROWS {
            let bytes = [
                nes.peek(address),
                nes.peek(address.wrapping_add(1)),
                nes.peek(address.wrapping_add(2)),
            ];
            let row = disasm::disassemble(address, bytes);
            disassembly.push(DisasmRow {
                address,
                text: row.mnemonic,
                current: i == 0,
            });
            address = address.wrapping_add(row.length as u16);
        }
        let (scroll_v, _, _) = nes.ppu.scroll_state();
        NesInspectState {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            s: cpu.s,
            p: cpu.p,
            pc: cpu.pc,
            scanline: nes.ppu.line(),
            dot: nes.ppu.dot(),
            ppu_control: nes.ppu.control,
            ppu_mask: nes.ppu.mask,
            ppu_status: nes.ppu.peek_status(),
            scroll_v,
            disassembly,
            frame: frame_count,
        }
    }

    fn register_groups(state: &NesInspectState) -> Vec<RegisterGroup> {
        cpu_register_groups(state)
    }

    fn sidebar_sections(state: &NesInspectState) -> Vec<Section> {
        nes_sidebar_sections(state)
    }

    fn snapshot(state: &NesInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(NesSnapshot::new(state))
    }

    fn running_status(state: &NesInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc.into(),
            sp: (state.s as u16 | 0x0100).into(),
            video_label: "PPU",
            video_summary: format!("scanline {} · dot {}", state.scanline, state.dot),
            frame,
        }
    }
}

/// The pad maps one-to-one onto the shared control ids and the console's
/// serial shift order (A, B, Select, Start, Up, Down, Left, Right).
const CONTROLLER_PORT: PortId = PortId(0);
const CONTROLLER: PeripheralId = PeripheralId(0);

const CONTROLLER_BUTTONS: &[ControlDescriptor] = &[
    ControlDescriptor::button(ControlRole::Start, "Start"),
    ControlDescriptor::button(ControlRole::Select, "Select"),
    ControlDescriptor::button(ControlRole::Action(0), "A"),
    ControlDescriptor::button(ControlRole::Action(1), "B"),
    ControlDescriptor::button(ControlRole::Up, "Up"),
    ControlDescriptor::button(ControlRole::Down, "Down"),
    ControlDescriptor::button(ControlRole::Left, "Left"),
    ControlDescriptor::button(ControlRole::Right, "Right"),
];

/// Only the first controller is wired; the second port and the expansion port
/// are not modelled.
pub const PORTS: &[PortDescriptor] = &[PortDescriptor {
    port: CONTROLLER_PORT,
    label: "Controller 1",
    accepts: &[PeripheralDescriptor {
        id: CONTROLLER,
        label: "Controller",
        provider: Provider::Console,
        controls: CONTROLLER_BUTTONS,
    }],
}];

fn apply_control(nes: &mut Nes, control: ControlId, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    if control.site != ControlSite::Port(CONTROLLER_PORT) {
        return;
    }
    let bit = match control.role {
        ControlRole::Action(0) => 0x01,
        ControlRole::Action(1) => 0x02,
        ControlRole::Select => 0x04,
        ControlRole::Start => 0x08,
        ControlRole::Up => 0x10,
        ControlRole::Down => 0x20,
        ControlRole::Left => 0x40,
        ControlRole::Right => 0x80,
        _ => return,
    };
    let mut state = nes.controller();
    if pressed {
        state |= bit;
    } else {
        state &= !bit;
    }
    nes.set_controller(state);
}

/// The canonical 2C02 palette (64 entries), approximated from the standard
/// NTSC values — a display-side stage, not a hardware claim.
fn nes_palette() -> Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<Arc<[RGB8]>> = OnceLock::new();
    PALETTE
        .get_or_init(|| {
            crate::ppu::master_palette()
                .iter()
                .map(|&(r, g, b)| RGB8 { r, g, b })
                .collect()
        })
        .clone()
}
