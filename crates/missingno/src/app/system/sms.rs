//! The Sega Master System's implementation of the system seam.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_gb::debugger::WatchCondition;
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::symbols::{Symbol, SymbolTable};
use missingno_gb::serial_transfer::SerialLink;
use missingno_sms::cartridge::CartridgeError;
use missingno_sms::console::Sms;
use missingno_sms::vdp::{self, Frame};
use rgb::RGB8;

use super::{ControlId, ControlInput, FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::debugger::sms::{SmsInspectState, SmsSnapshot};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{DisplayMode, FrameCapture, RgbaCapture};
use crate::app::screen::{IndexedFrame, ScreenDisplay};

pub const PLATFORM_NAME: &str = "Sega Master System";
pub const ROM_EXTENSIONS: &[&str] = &["sms"];

/// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);

/// Instruction budget per frame step; generous over the ~15k typical.
const FRAME_BUDGET: u32 = 200_000;

pub fn is_sms_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sms"))
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(SmsConsole {
        sms: Sms::new(rom)?,
        title,
        last_frame: blank_frame(),
    }))
}

struct SmsConsole {
    sms: Sms,
    title: String,
    last_frame: IndexedFrame,
}

fn blank_frame() -> IndexedFrame {
    IndexedFrame {
        width: vdp::PIXELS_PER_LINE as u32,
        height: vdp::ACTIVE_LINES as u32,
        pixels: vec![0; vdp::PIXELS_PER_LINE * vdp::ACTIVE_LINES as usize].into(),
        palette: cram_palette(&[0; 32]),
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

fn indexed_frame(frame: &Frame) -> IndexedFrame {
    IndexedFrame {
        width: vdp::PIXELS_PER_LINE as u32,
        height: vdp::ACTIVE_LINES as u32,
        pixels: frame.pixels.clone().into(),
        palette: cram_palette(&frame.cram),
    }
}

/// A display-ready RGBA screenshot of an indexed frame.
fn capture_indexed(frame: &IndexedFrame) -> FrameCapture {
    let mut data = Vec::with_capacity(frame.pixels.len() * 4);
    for &index in frame.pixels.iter() {
        let color = frame
            .palette
            .get(index as usize)
            .copied()
            .unwrap_or(RGB8::new(0, 0, 0));
        data.extend_from_slice(&[color.r, color.g, color.b, 255]);
    }
    FrameCapture {
        pixels: Vec::new(),
        sgb: None,
        display_mode: DisplayMode::Palette(String::new()),
        cgb_rgba: None,
        rgba: Some(RgbaCapture {
            width: frame.width,
            height: frame.height,
            data,
        }),
    }
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

impl SystemConsole for SmsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = self.sms.step_frame(FRAME_BUDGET).map(|frame| {
            self.last_frame = indexed_frame(&frame);
            ScreenDisplay::Indexed(self.last_frame.clone())
        });
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.sms.power_cycle();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.sms, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.sms.drain_audio_samples()
    }

    fn screen_display(&self) -> ScreenDisplay {
        ScreenDisplay::Indexed(self.last_frame.clone())
    }

    fn capture_frame(&self, _use_sgb_colors: bool, _palette_name: &str) -> FrameCapture {
        capture_indexed(&self.last_frame)
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }

    fn set_link(&mut self, _link: Box<dyn SerialLink>) {}

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Ok(Box::new(SmsDebugger::new(
            self.sms,
            self.title,
            self.last_frame,
        )))
    }
}

/// The SMS under the seam's debugger: stepping and breakpoints over the
/// console; symbols, code/data logging, and watchpoints have no backend
/// yet and report empty.
struct SmsDebugger {
    sms: Sms,
    breakpoints: BTreeSet<u16>,
    title: String,
    last_frame: IndexedFrame,
    inspect: SmsInspectState,
    symbols: Arc<SymbolTable>,
    frame_count: u64,
}

const CODE_WINDOW_ROWS: usize = 10;
const RUN_BUDGET: u32 = 400_000;

impl SmsDebugger {
    fn new(sms: Sms, title: String, last_frame: IndexedFrame) -> Self {
        let mut this = SmsDebugger {
            sms,
            breakpoints: BTreeSet::new(),
            title,
            last_frame,
            inspect: SmsInspectState::default(),
            symbols: Arc::new(SymbolTable::default()),
            frame_count: 0,
        };
        this.refresh();
        this
    }

    fn refresh(&mut self) {
        let sms = &self.sms;
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
        self.inspect = SmsInspectState {
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
            banks: [0, 1, 2].map(|slot| self.sms_bank(slot)),
            code_window,
            frame: self.frame_count,
        };
    }

    fn sms_bank(&self, slot: usize) -> u8 {
        // The mapper latches mirror into RAM, which inspection can read.
        self.sms.peek(0xFFFD + slot as u16)
    }

    fn display(&mut self, frame: Option<Frame>) -> Option<ScreenDisplay> {
        let frame = frame?;
        self.frame_count += 1;
        self.last_frame = indexed_frame(&frame);
        Some(ScreenDisplay::Indexed(self.last_frame.clone()))
    }

    fn at_breakpoint(&self) -> bool {
        self.breakpoints.contains(&self.sms.cpu.pc)
    }
}

impl SystemDebugger for SmsDebugger {
    fn step(&mut self) -> Option<ScreenDisplay> {
        self.sms.step_instruction();
        let frame = self.sms.take_frame();
        let display = self.display(frame);
        self.refresh();
        display
    }

    fn step_over(&mut self) -> Option<ScreenDisplay> {
        // CALL and RST push a return path; run to the next address.
        let opcode = self.sms.peek(self.sms.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        if !is_call {
            return self.step();
        }
        let return_address = self.sms.cpu.pc.wrapping_add(3);
        let mut frame = None;
        for _ in 0..RUN_BUDGET {
            self.sms.step_instruction();
            frame = self.sms.take_frame().or(frame);
            if self.sms.cpu.pc == return_address || self.at_breakpoint() {
                break;
            }
        }
        let display = self.display(frame);
        self.refresh();
        display
    }

    fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool) {
        let mut breakpoint_hit = false;
        let mut frame = None;
        for _ in 0..RUN_BUDGET {
            self.sms.step_instruction();
            if let Some(finished) = self.sms.take_frame() {
                frame = Some(finished);
                break;
            }
            if self.at_breakpoint() {
                breakpoint_hit = true;
                break;
            }
        }
        let display = self.display(frame);
        self.refresh();
        (display, breakpoint_hit)
    }

    fn reset(&mut self) {
        self.sms.power_cycle();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.sms, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.sms.drain_audio_samples()
    }

    fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    fn add_watchpoint(&mut self, _condition: WatchCondition) {}

    fn remove_watchpoint(&mut self, _condition: &WatchCondition) {}

    fn watchpoints(&self) -> &[WatchCondition] {
        &[]
    }

    fn last_watchpoint_hit(&self) -> Option<WatchCondition> {
        None
    }

    fn inspect(&self) -> &dyn Inspection {
        &self.inspect
    }

    fn pane_family(&self) -> &'static panes::Family {
        &panes::SMS_FAMILY
    }

    fn symbols(&self) -> Arc<SymbolTable> {
        self.symbols.clone()
    }

    fn set_symbols(&mut self, _symbols: SymbolTable) {}

    fn add_symbol(&mut self, _address: u16, _name: String) {}

    fn remove_symbol(&mut self, _symbol: &Symbol) {}

    fn save_symbols(&self, _path: &Path) {}

    fn cdl_window(&self) -> CdlWindow {
        CdlWindow::default()
    }

    fn load_cdl(&mut self, _path: &Path) {}

    fn save_cdl(&self, _path: &Path) {}

    fn snapshot(&self, frame: u64) -> DebugView {
        let mut state = self.inspect.clone();
        state.frame = frame;
        Box::new(SmsSnapshot::new(state))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.inspect.pc,
            sp: self.inspect.sp,
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", self.inspect.line, self.inspect.dot),
            frame,
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn capture_frame(&self, _use_sgb_colors: bool, _palette_name: &str) -> FrameCapture {
        capture_indexed(&self.last_frame)
    }

    fn capture_trace(&mut self, _path: &Path) -> Option<ScreenDisplay> {
        None
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(SmsConsole {
            sms: self.sms,
            title: self.title,
            last_frame: self.last_frame,
        })
    }
}
