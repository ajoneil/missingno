//! The NES / Famicom implementation of the system seam.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_6502::disasm;
use missingno_gb::debugger::WatchCondition;
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::symbols::{Symbol, SymbolTable};
use missingno_gb::serial_transfer::SerialLink;
use missingno_nes::cartridge::CartridgeError;
use missingno_nes::console::Nes;
use missingno_nes::ppu::{self, Frame};
use rgb::RGB8;

use super::{ControlId, ControlInput, FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::nes::{DisasmRow, NesInspectState, NesSnapshot};
use crate::app::debugger::panes;
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{DisplayMode, FrameCapture, RgbaCapture};
use crate::app::screen::{IndexedFrame, ScreenDisplay};

pub const PLATFORM_NAME: &str = "Nintendo Entertainment System";
pub const ROM_EXTENSIONS: &[&str] = &["nes"];

/// One NTSC frame: 262 lines × 341 dots ÷ 3 CPU cycles ≈ 29780 cycles.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_639);

/// CPU cycles per frame step, generous over the ~29.8k typical.
const FRAME_BUDGET: u32 = 200_000;

pub fn is_nes_rom(rom: &[u8]) -> bool {
    rom.len() >= 4 && &rom[0..4] == b"NES\x1A"
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(NesConsole {
        nes: Nes::new(rom)?,
        title,
        last_frame: blank_frame(),
    }))
}

struct NesConsole {
    nes: Nes,
    title: String,
    last_frame: IndexedFrame,
}

fn blank_frame() -> IndexedFrame {
    IndexedFrame {
        width: ppu::PIXELS_PER_LINE as u32,
        height: ppu::VISIBLE_LINES as u32,
        pixels: vec![0; ppu::PIXELS_PER_LINE * ppu::VISIBLE_LINES as usize].into(),
        palette: nes_palette(),
    }
}

fn indexed_frame(frame: &Frame) -> IndexedFrame {
    IndexedFrame {
        width: ppu::PIXELS_PER_LINE as u32,
        height: ppu::VISIBLE_LINES as u32,
        pixels: frame.pixels.clone().into(),
        palette: nes_palette(),
    }
}

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

impl SystemConsole for NesConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = self.nes.step_frame(FRAME_BUDGET).map(|frame| {
            self.last_frame = indexed_frame(&frame);
            ScreenDisplay::Indexed(self.last_frame.clone())
        });
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.nes.power_cycle();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.nes, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.nes.drain_audio_samples()
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
        Ok(Box::new(NesDebugger::new(
            self.nes,
            self.title,
            self.last_frame,
        )))
    }
}

/// The NES under the seam's debugger: stepping and breakpoints over the
/// console with real 6502 disassembly; symbols, code/data logging, and
/// watchpoints have no backend yet and report empty.
struct NesDebugger {
    nes: Nes,
    breakpoints: BTreeSet<u16>,
    title: String,
    last_frame: IndexedFrame,
    inspect: NesInspectState,
    symbols: Arc<SymbolTable>,
    frame_count: u64,
}

const DISASSEMBLY_ROWS: usize = 12;
const RUN_BUDGET: u32 = 400_000;
const JSR: u8 = 0x20;

impl NesDebugger {
    fn new(nes: Nes, title: String, last_frame: IndexedFrame) -> Self {
        let mut this = NesDebugger {
            nes,
            breakpoints: BTreeSet::new(),
            title,
            last_frame,
            inspect: NesInspectState::default(),
            symbols: Arc::new(SymbolTable::default()),
            frame_count: 0,
        };
        this.refresh();
        this
    }

    fn refresh(&mut self) {
        let cpu = &self.nes.cpu;
        let mut disassembly = Vec::with_capacity(DISASSEMBLY_ROWS);
        let mut address = cpu.pc;
        for i in 0..DISASSEMBLY_ROWS {
            let bytes = [
                self.nes.peek(address),
                self.nes.peek(address.wrapping_add(1)),
                self.nes.peek(address.wrapping_add(2)),
            ];
            let row = disasm::disassemble(address, bytes);
            disassembly.push(DisasmRow {
                address,
                text: row.mnemonic,
                current: i == 0,
            });
            address = address.wrapping_add(row.length as u16);
        }
        let (scroll_v, _, _) = self.nes.ppu.scroll_state();
        self.inspect = NesInspectState {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            s: cpu.s,
            p: cpu.p,
            pc: cpu.pc,
            scanline: self.nes.ppu.line(),
            dot: self.nes.ppu.dot(),
            ppu_control: self.nes.ppu.control,
            ppu_mask: self.nes.ppu.mask,
            ppu_status: self.nes.ppu.peek_status(),
            scroll_v,
            disassembly,
            frame: self.frame_count,
        };
    }

    fn display(&mut self, frame: Option<Frame>) -> Option<ScreenDisplay> {
        let frame = frame?;
        self.frame_count += 1;
        self.last_frame = indexed_frame(&frame);
        Some(ScreenDisplay::Indexed(self.last_frame.clone()))
    }

    fn at_breakpoint(&self) -> bool {
        self.breakpoints.contains(&self.nes.cpu.pc)
    }
}

impl SystemDebugger for NesDebugger {
    fn step(&mut self) -> Option<ScreenDisplay> {
        self.nes.step_instruction();
        let frame = self.nes.take_frame();
        let display = self.display(frame);
        self.refresh();
        display
    }

    fn step_over(&mut self) -> Option<ScreenDisplay> {
        if self.nes.peek(self.nes.cpu.pc) != JSR {
            return self.step();
        }
        let return_address = self.nes.cpu.pc.wrapping_add(3);
        let mut frame = None;
        for _ in 0..RUN_BUDGET {
            self.nes.step_instruction();
            frame = self.nes.take_frame().or(frame);
            if self.nes.cpu.pc == return_address || self.at_breakpoint() {
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
            self.nes.step_instruction();
            if let Some(finished) = self.nes.take_frame() {
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
        self.nes.power_cycle();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.nes, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.nes.drain_audio_samples()
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
        &panes::NES_FAMILY
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
        Box::new(NesSnapshot::new(state))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.inspect.pc,
            sp: self.inspect.s as u16 | 0x0100,
            video_label: "PPU",
            video_summary: format!(
                "scanline {} · dot {}",
                self.inspect.scanline, self.inspect.dot
            ),
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
        Box::new(NesConsole {
            nes: self.nes,
            title: self.title,
            last_frame: self.last_frame,
        })
    }
}

/// The canonical 2C02 palette (64 entries), approximated from the standard
/// NTSC values — a display-side stage, not a hardware claim.
fn nes_palette() -> Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<Arc<[RGB8]>> = OnceLock::new();
    PALETTE
        .get_or_init(|| {
            const RGB: [(u8, u8, u8); 64] = [
                (84, 84, 84),
                (0, 30, 116),
                (8, 16, 144),
                (48, 0, 136),
                (68, 0, 100),
                (92, 0, 48),
                (84, 4, 0),
                (60, 24, 0),
                (32, 42, 0),
                (8, 58, 0),
                (0, 64, 0),
                (0, 60, 0),
                (0, 50, 60),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (152, 150, 152),
                (8, 76, 196),
                (48, 50, 236),
                (92, 30, 228),
                (136, 20, 176),
                (160, 20, 100),
                (152, 34, 32),
                (120, 60, 0),
                (84, 90, 0),
                (40, 114, 0),
                (8, 124, 0),
                (0, 118, 40),
                (0, 102, 120),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (236, 238, 236),
                (76, 154, 236),
                (120, 124, 236),
                (176, 98, 236),
                (228, 84, 236),
                (236, 88, 180),
                (236, 106, 100),
                (212, 136, 32),
                (160, 170, 0),
                (116, 196, 0),
                (76, 208, 32),
                (56, 204, 108),
                (56, 180, 204),
                (60, 60, 60),
                (0, 0, 0),
                (0, 0, 0),
                (236, 238, 236),
                (168, 204, 236),
                (188, 188, 236),
                (212, 178, 236),
                (236, 174, 236),
                (236, 174, 212),
                (236, 180, 176),
                (228, 196, 144),
                (204, 210, 120),
                (180, 222, 120),
                (168, 226, 144),
                (152, 226, 180),
                (160, 214, 228),
                (160, 162, 160),
                (0, 0, 0),
                (0, 0, 0),
            ];
            RGB.iter()
                .map(|&(r, g, b)| RGB8::new(r, g, b))
                .collect::<Vec<_>>()
                .into()
        })
        .clone()
}
