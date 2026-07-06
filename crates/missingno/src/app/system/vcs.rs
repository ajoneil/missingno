//! The Atari 2600's implementation of the system seam. Emulator-only for
//! now: the family reports no debugger backend, so the shell falls back to
//! plain emulation.

use std::time::Duration;

use missingno_gb::serial_transfer::SerialLink;
use missingno_vcs::cartridge::CartridgeError;
use missingno_vcs::console::{JoystickDirection, Vcs};
use missingno_vcs::tia::VISIBLE_CLOCKS;
use rgb::RGB8;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use missingno_gb::debugger::WatchCondition;
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::symbols::{Symbol, SymbolTable};
use missingno_vcs::console::Frame;
use missingno_vcs::cpu::disasm;

use super::{ControlId, ControlInput, FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::debugger::vcs::{DisasmRow, VcsInspectState, VcsSnapshot};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{DisplayMode, FrameCapture, RgbaCapture};
use crate::app::screen::{IndexedFrame, ScreenDisplay};

pub const PLATFORM_NAME: &str = "Atari 2600";
pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// Nominal NTSC frame: 262 lines × 228 clocks at the 3.579545 MHz colour
/// clock. Kernels vary line counts; the pacing loop uses the convention.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_684);

/// Frames are emergent from VSYNC; bound the search so a kernel that never
/// syncs cannot stall the emulation thread.
const FRAME_BUDGET_LINES: usize = 1000;

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide).
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => matches!(rom.len(), 0x800 | 0x1000),
        _ => false,
    }
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(VcsConsole {
        vcs: Vcs::new(rom)?,
        title,
        last_frame: blank_frame(),
    }))
}

struct VcsConsole {
    vcs: Vcs,
    title: String,
    last_frame: IndexedFrame,
}

fn indexed_frame(frame: &Frame) -> IndexedFrame {
    let height = frame.lines.len() as u32;
    let mut pixels = Vec::with_capacity(frame.lines.len() * VISIBLE_CLOCKS);
    for line in &frame.lines {
        // TIA colour bytes drop bit 0; the palette is 7-bit indexed.
        pixels.extend(line.iter().map(|&p| p >> 1));
    }
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height,
        pixels: pixels.into(),
        palette: ntsc_palette(),
    }
}

fn blank_frame() -> IndexedFrame {
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height: 192,
        pixels: vec![0; VISIBLE_CLOCKS * 192].into(),
        palette: ntsc_palette(),
    }
}

impl SystemConsole for VcsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = self.vcs.step_frame(FRAME_BUDGET_LINES).map(|frame| {
            self.last_frame = indexed_frame(&frame);
            ScreenDisplay::Indexed(self.last_frame.clone())
        });
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.vcs.power_cycle();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.vcs, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.vcs.drain_audio_samples()
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
        Ok(Box::new(VcsDebugger::new(
            missingno_vcs::debugger::Debugger::new(self.vcs),
            self.title,
            self.last_frame,
        )))
    }
}

/// Paddle 0's knob rides the first analog control id.
pub const PADDLE_CONTROL: ControlId = ControlId(8);

/// The family's reading of the shared control ids: the standard pad maps
/// onto the joystick and fire, Start/Select work the console switches,
/// and the paddle takes the axis.
fn apply_control(vcs: &mut Vcs, control: ControlId, input: ControlInput) {
    match input {
        ControlInput::Digital(pressed) => {
            let direction = match control.0 {
                0 => return vcs.set_console_reset(pressed),
                1 => return vcs.set_console_select(pressed),
                2 | 3 => return vcs.set_fire(pressed),
                4 => JoystickDirection::Up,
                5 => JoystickDirection::Down,
                6 => JoystickDirection::Left,
                7 => JoystickDirection::Right,
                _ => return,
            };
            vcs.set_joystick(direction, pressed);
        }
        ControlInput::Axis(value) => {
            if control == PADDLE_CONTROL {
                vcs.set_paddle(0, value);
            }
        }
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

/// The 128-colour NTSC TIA palette (colour byte bits 7-1: hue 4, luma 3),
/// approximated from hue-angle chroma — a display-side calibratable stage,
/// not a hardware claim.
fn ntsc_palette() -> std::sync::Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<std::sync::Arc<[RGB8]>> = OnceLock::new();
    PALETTE
        .get_or_init(|| {
            let mut palette = [RGB8::new(0, 0, 0); 128];
            for (index, entry) in palette.iter_mut().enumerate() {
                let hue = (index >> 3) & 0x0F;
                let luma = (index & 0x07) as f32;
                let y = 0.12 + 0.85 * (luma / 7.0);
                let (i, q) = if hue == 0 {
                    (0.0, 0.0)
                } else {
                    // Hue 1 starts gold and the phase walks the colour wheel.
                    let angle = (103.0 - 25.7 * (hue as f32 - 1.0)).to_radians();
                    let saturation = 0.28 - 0.02 * (luma / 7.0);
                    (saturation * angle.cos(), saturation * angle.sin())
                };
                let r = y + 0.956 * i + 0.619 * q;
                let g = y - 0.272 * i - 0.647 * q;
                let b = y - 1.106 * i + 1.703 * q;
                *entry = RGB8::new(channel(r), channel(g), channel(b));
            }
            palette.into()
        })
        .clone()
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0) as u8
}

/// The VCS under its debugging backend, adapted to the seam. Symbols,
/// code/data logging, and watchpoints have no backend yet: those seam
/// methods accept and report nothing.
struct VcsDebugger {
    core: missingno_vcs::debugger::Debugger,
    title: String,
    last_frame: IndexedFrame,
    inspect: VcsInspectState,
    symbols: Arc<SymbolTable>,
    frame_count: u64,
}

/// Disassembly rows shown from the current instruction forward.
const DISASSEMBLY_ROWS: usize = 12;

impl VcsDebugger {
    fn new(
        core: missingno_vcs::debugger::Debugger,
        title: String,
        last_frame: IndexedFrame,
    ) -> Self {
        let mut this = VcsDebugger {
            core,
            title,
            last_frame,
            inspect: VcsInspectState::default(),
            symbols: Arc::new(SymbolTable::default()),
            frame_count: 0,
        };
        this.refresh();
        this
    }

    /// Rebuild the inspection state from the console (peek-only).
    fn refresh(&mut self) {
        let vcs = self.core.console();
        let cpu = &vcs.cpu;
        let mut disassembly = Vec::with_capacity(DISASSEMBLY_ROWS);
        let mut address = cpu.pc;
        for i in 0..DISASSEMBLY_ROWS {
            let bytes = [
                vcs.peek(address),
                vcs.peek(address.wrapping_add(1)),
                vcs.peek(address.wrapping_add(2)),
            ];
            let row = disasm::disassemble(address, bytes);
            disassembly.push(DisasmRow {
                address,
                text: row.mnemonic,
                current: i == 0,
            });
            address = address.wrapping_add(row.length as u16);
        }
        self.inspect = VcsInspectState {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            s: cpu.s,
            p: cpu.p,
            pc: cpu.pc,
            beam: vcs.tia.beam(),
            scanline: vcs.scanline(),
            timer: vcs.peek(0x0284),
            timer_underflowed: vcs.peek(0x0285) & 0x80 != 0,
            swcha: vcs.peek(0x0280),
            swchb: vcs.peek(0x0282),
            collisions: std::array::from_fn(|i| vcs.peek(i as u16)),
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
}

impl SystemDebugger for VcsDebugger {
    fn step(&mut self) -> Option<ScreenDisplay> {
        let frame = self.core.step();
        let display = self.display(frame);
        self.refresh();
        display
    }

    fn step_over(&mut self) -> Option<ScreenDisplay> {
        let (frame, _) = self.core.step_over();
        let display = self.display(frame);
        self.refresh();
        display
    }

    fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool) {
        let (frame, stop) = self.core.step_frame();
        let display = self.display(frame);
        self.refresh();
        (display, stop == missingno_vcs::debugger::Stop::Breakpoint)
    }

    fn reset(&mut self) {
        self.core.console_mut().power_cycle();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(self.core.console_mut(), control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.core.console_mut().drain_audio_samples()
    }

    fn set_breakpoint(&mut self, address: u16) {
        self.core.set_breakpoint(address);
    }

    fn clear_breakpoint(&mut self, address: u16) {
        self.core.clear_breakpoint(address);
    }

    fn breakpoints(&self) -> &BTreeSet<u16> {
        self.core.breakpoints()
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
        &panes::VCS_FAMILY
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
        Box::new(VcsSnapshot::new(state))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.inspect.pc,
            sp: self.inspect.s as u16 | 0x0100,
            video_label: "TIA",
            video_summary: format!(
                "beam {} · line {}",
                self.inspect.beam, self.inspect.scanline
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
        Box::new(VcsConsole {
            vcs: self.core.into_console(),
            title: self.title,
            last_frame: self.last_frame,
        })
    }
}
