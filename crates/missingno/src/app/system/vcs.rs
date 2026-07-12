//! The Atari VCS's implementation of the system seam.

use std::time::Duration;

use missingno_vcs::TvStandard;
use missingno_vcs::cartridge::CartridgeError;
use missingno_vcs::console::{JoystickDirection, Vcs};
use missingno_vcs::tia::{VISIBLE_CLOCKS, palette_index};
use missingno_vcs::tv_standard::PIXEL_ASPECT;
use rgb::RGB8;

use std::collections::BTreeSet;

use missingno_vcs::console::{Frame, Scanline};
use missingno_vcs::cpu::disasm;

use super::{ConsoleSwitch, ControlId, ControlInput, FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::debugger::vcs::{DisasmRow, VcsInspectState, VcsSnapshot};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{CaptureOptions, FrameCapture};
use crate::app::screen::{IndexedFrame, ScreenDisplay};

pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// The family's names for the shared control ids, indexed by id.
/// Start/Select work the console switches; both buttons fire.
pub const CONTROL_LABELS: [&str; 8] = [
    "Reset", "Select", "Fire", "Fire", "Up", "Down", "Left", "Right",
];

/// The latching console switches, driven through control ids past the
/// paddle (id 8). Positions and defaults match the RIOT's SWCHB state.
pub const CONSOLE_SWITCHES: [ConsoleSwitch; 3] = [
    ConsoleSwitch {
        control: ControlId(9),
        label: "Left Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(10),
        label: "Right Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(11),
        label: "TV Type",
        positions: ["B•W", "Color"],
        default_high: true,
    },
];

/// Nominal NTSC frame: 262 lines × 228 clocks at the 3.579545 MHz colour
/// clock. Kernels vary line counts; the pacing loop uses the convention.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_684);

/// Frames are emergent from VSYNC; bound the search so a kernel that never
/// syncs cannot stall the emulation thread.
const FRAME_BUDGET_LINES: usize = 1000;

/// Scanlines of asserted VSYNC the television integrates before the field
/// re-anchors. The console drives VSYNC as a plain latch; this lock lives in
/// the set (off-chip) and is calibratable — reference emulators model 2 and the
/// safe kernel convention is 3, so anything shorter is swallowed.
const VSYNC_LOCK_LINES: usize = 2;

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
    // Region detection (ROM-hash → standard) is a future game-db concern; the
    // frontend runs NTSC until then. Pacing, aspect, and palette follow suit.
    Ok(Box::new(VcsConsole {
        vcs: Vcs::new(rom, TvStandard::Ntsc)?,
        title,
        last_frame: blank_frame(),
        tv: Television::new(),
    }))
}

struct VcsConsole {
    vcs: Vcs,
    title: String,
    last_frame: IndexedFrame,
    tv: Television,
}

/// The picture window shown from the full field the core emits: skip the
/// VBLANK lead-in after VSYNC, then show a fixed height so on-screen
/// geometry stays stable across kernels of varying line count. Values are
/// the standard NTSC/PAL picture regions (a TV crops to roughly this).
/// Frontend-only — the core keeps emitting every scanline.
struct DisplayWindow {
    skip: usize,
    height: usize,
}

fn display_window(standard: TvStandard) -> DisplayWindow {
    match standard {
        TvStandard::Ntsc => DisplayWindow {
            skip: 23,
            height: 228,
        },
        TvStandard::Pal => DisplayWindow {
            skip: 32,
            height: 274,
        },
    }
}

fn indexed_frame(frame: &Frame, standard: TvStandard) -> IndexedFrame {
    let window = display_window(standard);
    let black = palette_index(0) as u8;
    let mut pixels = vec![black; window.height * VISIBLE_CLOCKS];
    for row in 0..window.height {
        if let Some(line) = frame.lines.get(window.skip + row) {
            let dst = row * VISIBLE_CLOCKS;
            for (i, &p) in line.iter().enumerate() {
                pixels[dst + i] = palette_index(p) as u8;
            }
        }
    }
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height: window.height as u32,
        pixels: pixels.into(),
        palette: ntsc_palette(),
        pixel_aspect: PIXEL_ASPECT,
    }
}

fn blank_frame() -> IndexedFrame {
    let height = display_window(TvStandard::Ntsc).height as u32;
    IndexedFrame::blank(VISIBLE_CLOCKS as u32, height, PIXEL_ASPECT, ntsc_palette())
}

/// The television's vertical-sync separator. A real set integrates the incoming
/// composite sync and only retraces — re-anchoring the field — once VSYNC has
/// been asserted across `VSYNC_LOCK_LINES` scanlines; a briefer pulse never
/// charges the integrator and is swallowed, leaving the field timing unchanged.
/// The console just drives the VSYNC pin (a plain latch); this lock is off-chip.
struct Television {
    building: Vec<[u8; VISIBLE_CLOCKS]>,
    vsync_run: usize,
}

impl Television {
    fn new() -> Self {
        Television {
            building: Vec::new(),
            vsync_run: 0,
        }
    }

    /// Feed one scanline. Returns the completed field when the integrator locks
    /// on a VSYNC assertion that has persisted the threshold — that boundary is
    /// the field's end; the VSYNC lines themselves are the sync interval, not
    /// picture, so they are never part of the field.
    fn feed(&mut self, line: Scanline) -> Option<Frame> {
        if line.vsync {
            self.vsync_run += 1;
            if self.vsync_run == VSYNC_LOCK_LINES && !self.building.is_empty() {
                return Some(Frame {
                    lines: std::mem::take(&mut self.building),
                });
            }
            None
        } else {
            self.vsync_run = 0;
            self.building.push(line.pixels);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_line(marker: u8) -> Scanline {
        Scanline {
            pixels: [marker; VISIBLE_CLOCKS],
            vsync: false,
        }
    }

    fn vsync_line() -> Scanline {
        Scanline {
            pixels: [0; VISIBLE_CLOCKS],
            vsync: true,
        }
    }

    /// Drive 200 picture lines, a stray VSYNC pulse of `pulse` lines, 40 more
    /// picture lines, then a full 3-line VSYNC. Return each completed field's
    /// line count. A swallowed pulse yields one merged field before the final
    /// VSYNC; a locking pulse splits the field there.
    fn run(pulse: usize) -> Vec<usize> {
        let mut lines = Vec::new();
        lines.extend((0..200).map(|_| picture_line(1)));
        lines.extend((0..pulse).map(|_| vsync_line()));
        lines.extend((0..40).map(|_| picture_line(2)));
        lines.extend((0..3).map(|_| vsync_line()));

        let mut tv = Television::new();
        let mut fields = Vec::new();
        for line in lines {
            if let Some(frame) = tv.feed(line) {
                fields.push(frame.lines.len());
            }
        }
        fields
    }

    #[test]
    fn sub_threshold_vsync_is_swallowed() {
        // A 1-line pulse never locks: the field spans across it and re-anchors
        // only at the following 3-line VSYNC — one merged 240-line field.
        assert_eq!(run(1), vec![240]);
    }

    #[test]
    fn threshold_vsync_re_anchors() {
        // A 3-line pulse locks: the field ends at the pulse (200 lines); the
        // trailing 40 picture lines then form the next field at the final VSYNC.
        assert_eq!(run(3), vec![200, 40]);
    }

    #[test]
    fn exactly_two_lines_locks() {
        // The threshold itself: a 2-line pulse locks and splits, same as three.
        assert_eq!(run(2), vec![200, 40]);
    }
}

impl SystemConsole for VcsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let standard = self.vcs.tv_standard();
        // Drive the console scanline by scanline through the television, which
        // integrates VSYNC to decide the field. Bounded so a kernel that never
        // syncs cannot stall the emulation thread.
        let mut display = None;
        for _ in 0..FRAME_BUDGET_LINES {
            let line = self.vcs.step_scanline();
            if let Some(frame) = self.tv.feed(line) {
                self.last_frame = indexed_frame(&frame, standard);
                display = Some(ScreenDisplay::Indexed(self.last_frame.clone()));
                break;
            }
        }
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

    fn console_switches(&self) -> &'static [ConsoleSwitch] {
        &CONSOLE_SWITCHES
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.vcs.drain_audio_samples()
    }

    fn screen_display(&self) -> ScreenDisplay {
        ScreenDisplay::Indexed(self.last_frame.clone())
    }

    fn capture_frame(&self, _options: &CaptureOptions) -> FrameCapture {
        FrameCapture::from_indexed(&self.last_frame)
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

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
                // Latching console switches carry their level, not a press.
                9 => return vcs.set_difficulty(0, pressed),
                10 => return vcs.set_difficulty(1, pressed),
                11 => return vcs.set_color_mode(pressed),
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

/// The core's NTSC TIA palette as the screen path's shared RGB8 slice.
fn ntsc_palette() -> std::sync::Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<std::sync::Arc<[RGB8]>> = OnceLock::new();
    PALETTE
        .get_or_init(|| {
            missingno_vcs::tia::palette(TvStandard::Ntsc)
                .iter()
                .map(|&(r, g, b)| RGB8::new(r, g, b))
                .collect::<Vec<_>>()
                .into()
        })
        .clone()
}

/// The VCS under its debugging backend, adapted to the seam. Symbols,
/// code/data logging, and watchpoints have no backend yet — the seam
/// defaults report them absent.
struct VcsDebugger {
    core: missingno_vcs::debugger::Debugger,
    title: String,
    last_frame: IndexedFrame,
    inspect: VcsInspectState,
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
        let standard = self.core.console().tv_standard();
        self.last_frame = indexed_frame(&frame, standard);
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

    fn inspect(&self) -> &dyn Inspection {
        &self.inspect
    }

    fn pane_family(&self) -> &'static panes::Family {
        &panes::VCS_FAMILY
    }

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

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn capture_frame(&self, _options: &CaptureOptions) -> FrameCapture {
        FrameCapture::from_indexed(&self.last_frame)
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(VcsConsole {
            vcs: self.core.into_console(),
            title: self.title,
            last_frame: self.last_frame,
            tv: Television::new(),
        })
    }
}
