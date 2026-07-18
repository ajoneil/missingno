//! The NES family's debugger seam: its inspection state and the
//! stepping-system binding over the console. One owned state struct serves
//! both the paused view (refreshed after every step) and the per-frame
//! snapshot the running view renders from.

use std::sync::Arc;
use std::time::Duration;

use missingno_6502::disasm;
use missingno_core::stepping::SteppingSystem;
use missingno_core::system::{ControlId, ControlInput, DebugView, InspectSnapshot, RunningStatus};
use missingno_core::video::IndexedFrame;
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
}

/// iNES media carries the `NES\x1A` magic in its first four bytes.
pub fn is_nes_rom(rom: &[u8]) -> bool {
    rom.len() >= 4 && &rom[0..4] == b"NES\x1A"
}

pub struct NesSystem;

impl SteppingSystem for NesSystem {
    type Core = Nes;
    type Frame = Frame;
    type InspectState = NesInspectState;

    /// One NTSC frame: 262 lines × 341 dots ÷ 3 CPU cycles ≈ 29780 cycles.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_639);
    const RUN_BUDGET: u32 = 400_000;
    const PIXEL_ASPECT: f32 = PIXEL_ASPECT;

    fn pc(nes: &Nes) -> u16 {
        nes.cpu.pc
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

    fn drain_audio_samples(nes: &mut Nes) -> Vec<(f32, f32)> {
        nes.drain_audio_samples()
    }

    fn indexed_frame(frame: &Frame) -> IndexedFrame {
        IndexedFrame {
            width: ppu::PIXELS_PER_LINE as u32,
            height: ppu::VISIBLE_LINES as u32,
            pixels: frame.pixels.clone().into(),
            palette: nes_palette(),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn blank_frame() -> IndexedFrame {
        IndexedFrame::blank(
            ppu::PIXELS_PER_LINE as u32,
            ppu::VISIBLE_LINES as u32,
            PIXEL_ASPECT,
            nes_palette(),
        )
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
fn apply_control(nes: &mut Nes, control: ControlId, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    let bit = match control.0 {
        2 => 0x01, // A
        3 => 0x02, // B
        1 => 0x04, // Select
        0 => 0x08, // Start
        4 => 0x10, // Up
        5 => 0x20, // Down
        6 => 0x40, // Left
        7 => 0x80, // Right
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
