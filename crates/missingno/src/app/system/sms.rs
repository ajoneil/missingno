//! The Sega Master System's implementation of the system seam, as a
//! [`SteppingSystem`] over the console.

use std::sync::Arc;
use std::time::Duration;

use missingno_sms::cartridge::CartridgeError;
use missingno_sms::console::Sms;
use missingno_sms::vdp::{self, Frame};
use rgb::RGB8;

use super::stepping::{SteppingConsole, SteppingSystem};
use super::{ControlId, ControlInput, SystemConsole};
use crate::app::debugger::inspect::DebugView;
use crate::app::debugger::panes;
use crate::app::debugger::sms::{SmsInspectState, SmsSnapshot};
use crate::app::emu_thread::RunningStatus;
use crate::app::screen::IndexedFrame;

pub const PLATFORM_NAME: &str = "Sega Master System";
pub const ROM_EXTENSIONS: &[&str] = &["sms"];

/// Instruction budget per frame step; generous over the ~15k typical.
const FRAME_BUDGET: u32 = 200_000;

const CODE_WINDOW_ROWS: usize = 10;

pub fn is_sms_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sms"))
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(SteppingConsole::<SmsSystem>::new(
        Sms::new(rom)?,
        title,
    )))
}

pub struct SmsSystem;

impl SteppingSystem for SmsSystem {
    type Core = Sms;
    type Frame = Frame;
    type InspectState = SmsInspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    const RUN_BUDGET: u32 = 400_000;

    fn pane_family() -> &'static panes::Family {
        &panes::SMS_FAMILY
    }

    fn pc(sms: &Sms) -> u16 {
        sms.cpu.pc
    }

    fn step_instruction(sms: &mut Sms) {
        sms.step_instruction();
    }

    fn take_frame(sms: &mut Sms) -> Option<Frame> {
        sms.take_frame()
    }

    fn step_frame(sms: &mut Sms) -> Option<Frame> {
        sms.step_frame(FRAME_BUDGET)
    }

    fn power_cycle(sms: &mut Sms) {
        sms.power_cycle();
    }

    fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
        apply_control(sms, control, input);
    }

    fn drain_audio_samples(sms: &mut Sms) -> Vec<(f32, f32)> {
        sms.drain_audio_samples()
    }

    fn indexed_frame(frame: &Frame) -> IndexedFrame {
        IndexedFrame {
            width: vdp::PIXELS_PER_LINE as u32,
            height: vdp::ACTIVE_LINES as u32,
            pixels: frame.pixels.clone().into(),
            palette: cram_palette(&frame.cram),
        }
    }

    fn blank_frame() -> IndexedFrame {
        IndexedFrame::blank(
            vdp::PIXELS_PER_LINE as u32,
            vdp::ACTIVE_LINES as u32,
            cram_palette(&[0; 32]),
        )
    }

    fn step_over_target(sms: &Sms) -> Option<u16> {
        // CALL and RST push a return path; run to the next address.
        let opcode = sms.peek(sms.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        is_call.then(|| sms.cpu.pc.wrapping_add(3))
    }

    fn inspect(sms: &Sms, frame_count: u64) -> SmsInspectState {
        let cpu = &sms.cpu;
        let mut code_window = Vec::with_capacity(CODE_WINDOW_ROWS);
        let mut address = cpu.pc;
        for _ in 0..CODE_WINDOW_ROWS {
            code_window.push((
                address,
                [
                    sms.peek(address),
                    sms.peek(address.wrapping_add(1)),
                    sms.peek(address.wrapping_add(2)),
                    sms.peek(address.wrapping_add(3)),
                ],
            ));
            address = address.wrapping_add(4);
        }
        // The mapper latches mirror into RAM, which inspection can read.
        let banks = [0, 1, 2].map(|slot| sms.peek(0xFFFD + slot as u16));
        SmsInspectState {
            a: cpu.a,
            f: cpu.f,
            bc: cpu.bc(),
            de: cpu.de(),
            hl: cpu.hl(),
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
            line: sms.vdp.line(),
            dot: sms.vdp.dot(),
            vdp_status: sms.vdp.peek_status(),
            vdp_registers: sms.vdp.registers,
            banks,
            code_window,
            frame: frame_count,
        }
    }

    fn snapshot(state: &SmsInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(SmsSnapshot::new(state))
    }

    fn running_status(state: &SmsInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc,
            sp: state.sp,
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.line, state.dot),
            frame,
        }
    }
}

/// Resolve a CRAM snapshot (6-bit --BBGGRR) to display RGB.
fn cram_palette(cram: &[u8; 32]) -> Arc<[RGB8]> {
    cram.iter()
        .map(|&entry| {
            let channel = |bits: u8| (bits & 0x03) * 85;
            RGB8::new(channel(entry), channel(entry >> 2), channel(entry >> 4))
        })
        .collect()
}

/// The family's reading of the shared control ids: the pad maps onto the
/// port lines, and Start works the console Pause button (an NMI).
fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    let line = match control.0 {
        0 => {
            if pressed {
                sms.cpu.trigger_nmi();
            }
            return;
        }
        2 => 0x10, // button 1
        3 => 0x20, // button 2
        4 => 0x01, // up
        5 => 0x02, // down
        6 => 0x04, // left
        7 => 0x08, // right
        _ => return,
    };
    if pressed {
        sms.port_dc &= !line;
    } else {
        sms.port_dc |= line;
    }
}
