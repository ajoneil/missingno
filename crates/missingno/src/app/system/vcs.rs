//! The Atari VCS's implementation of the system seam.

use std::time::Duration;

use missingno_vcs::CartType;
use missingno_vcs::TvStandard;
use missingno_vcs::cartridge::CartridgeError;
use missingno_vcs::console::{JoystickDirection, Vcs};
use missingno_vcs::tia::{VISIBLE_CLOCKS, palette_index};
use missingno_vcs::tv_standard::PIXEL_ASPECT;
use rgb::RGB8;

use std::collections::BTreeSet;

use missingno_vcs::console::Frame;
use missingno_vcs::cpu::disasm;

use super::{
    ConsoleSwitch, ControlId, ControlInput, FrameOutcome, StepOutcome, SystemConsole,
    SystemDebugger,
};
use crate::app::debugger::inspect::DebugView;
use crate::app::debugger::vcs::{DisasmRow, VcsInspectState, VcsSnapshot};
use crate::app::emu_thread::RunningStatus;
use crate::app::screen::IndexedFrame;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::video::{self, Frame as VideoFrame, Television, VideoOut};

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

/// Nominal frame: a full field of 228-clock lines at the colour clock — 262
/// lines (NTSC) or 312 (PAL). Kernels vary line counts; pacing uses the
/// convention so the frame rate follows the broadcast standard.
fn frame_interval(standard: TvStandard) -> Duration {
    let lines = match standard {
        TvStandard::Ntsc => 262.0,
        TvStandard::Pal | TvStandard::Secam => 312.0,
    };
    Duration::from_secs_f32(lines * 228.0 / missingno_vcs::tv_standard::master_clock_hz(standard))
}

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

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<super::TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    // The library's metadata is authoritative; carts carry no region header and
    // the size heuristic can't always name the board, so fall back only when
    // the game-db is silent — then probe the standard from the ROM's own field
    // length. Pacing, aspect, and palette follow the standard.
    let cart = cart_type.and_then(core_cart_type);
    let region = match tv_standard {
        Some(standard) => standard,
        None => probe_tv_standard(rom, cart),
    };
    Ok(Box::new(VcsConsole {
        vcs: Vcs::new(rom, region, cart)?,
        title,
        last_frame: blank_frame(),
        tv: Television::new(VSYNC_LOCK_LINES),
    }))
}

/// Scanlines per field that split NTSC (~262) from PAL (~312); the midpoint
/// clears the ~284–290 overlap where a handful of ROMs are genuinely ambiguous.
const NTSC_PAL_FIELD_THRESHOLD: usize = 287;

/// Detect an uncatalogued ROM's broadcast standard by counting scanlines per
/// field: a PAL field runs ~50 lines longer than NTSC. The standard only scales
/// the master clock, not the kernel's line count, so a provisional NTSC build
/// reads the field length truthfully.
fn probe_tv_standard(rom: &[u8], cart_type: Option<CartType>) -> TvStandard {
    let Ok(mut vcs) = Vcs::new(rom, TvStandard::Ntsc, cart_type) else {
        return TvStandard::Ntsc;
    };
    let mut tv = Television::<VISIBLE_CLOCKS>::new(VSYNC_LOCK_LINES);
    let mut fields = Vec::new();
    let mut lines_this_field = 0usize;
    // A few fields, bounded so a kernel that never syncs can't spin.
    for _ in 0..(FRAME_BUDGET_LINES * 8) {
        let line = vcs.step_scanline();
        lines_this_field += 1;
        if tv
            .feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            })
            .is_some()
        {
            fields.push(lines_this_field);
            lines_this_field = 0;
            if fields.len() >= 6 {
                break;
            }
        }
    }
    classify_fields(&fields)
}

/// Classify measured field lengths by their median (robust to a long startup
/// field), skipping the first warm-up field; NTSC when nothing synced.
fn classify_fields(fields: &[usize]) -> TvStandard {
    let mut steady: Vec<usize> = fields.iter().copied().skip(1).collect();
    if steady.is_empty() {
        return TvStandard::Ntsc;
    }
    steady.sort_unstable();
    if steady[steady.len() / 2] > NTSC_PAL_FIELD_THRESHOLD {
        TvStandard::Pal
    } else {
        TvStandard::Ntsc
    }
}

/// Parse a game-db board code into the core's board type; codes the core can't
/// build yet return `None`, leaving `Cartridge::load` to size-detect.
fn core_cart_type(code: &str) -> Option<CartType> {
    match code {
        "2K" => Some(CartType::Plain2K),
        "4K" => Some(CartType::Plain4K),
        "F8" => Some(CartType::F8),
        "F8SC" => Some(CartType::F8Sc),
        "F6" => Some(CartType::F6),
        "F6SC" => Some(CartType::F6Sc),
        "F4" => Some(CartType::F4),
        "F4SC" => Some(CartType::F4Sc),
        "FA" => Some(CartType::Fa),
        "FC" => Some(CartType::Fc),
        "FE" => Some(CartType::Fe),
        "E0" => Some(CartType::E0),
        "E7" => Some(CartType::E7),
        "CV" => Some(CartType::Cv),
        "UA" => Some(CartType::Ua),
        "3F" => Some(CartType::ThreeF),
        "3E" => Some(CartType::ThreeE),
        "3E+" => Some(CartType::ThreeEPlus),
        "DPC" => Some(CartType::Dpc),
        "AR" => Some(CartType::Ar),
        "F0" => Some(CartType::F0),
        "JANE" => Some(CartType::Jane),
        "WF8" => Some(CartType::Wf8),
        "WD" => Some(CartType::Wd),
        "0FA0" => Some(CartType::ZeroFa0),
        "03E0" => Some(CartType::Zero3E0),
        "0840" => Some(CartType::Zero840),
        "EF" => Some(CartType::Ef),
        "DF" => Some(CartType::Df),
        "BF" => Some(CartType::Bf),
        "SB" => Some(CartType::Sb),
        "X07" => Some(CartType::X07),
        "MDM" => Some(CartType::Mdm),
        _ => None,
    }
}

struct VcsConsole {
    vcs: Vcs,
    title: String,
    last_frame: IndexedFrame,
    tv: Television<VISIBLE_CLOCKS>,
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
        // SECAM shares PAL's 50 Hz, 312-line field geometry.
        TvStandard::Pal | TvStandard::Secam => DisplayWindow {
            skip: 32,
            height: 274,
        },
    }
}

fn indexed_frame(lines: &[[u8; VISIBLE_CLOCKS]], standard: TvStandard) -> IndexedFrame {
    let window = display_window(standard);
    let black = palette_index(0) as u8;
    let mut pixels = vec![black; window.height * VISIBLE_CLOCKS];
    for row in 0..window.height {
        if let Some(line) = lines.get(window.skip + row) {
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
        palette: region_palette(standard),
        pixel_aspect: PIXEL_ASPECT,
    }
}

fn blank_frame() -> IndexedFrame {
    let height = display_window(TvStandard::Ntsc).height as u32;
    IndexedFrame::blank(
        VISIBLE_CLOCKS as u32,
        height,
        PIXEL_ASPECT,
        region_palette(TvStandard::Ntsc),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_fields_splits_ntsc_from_pal() {
        assert_eq!(classify_fields(&[42, 262, 262, 262]), TvStandard::Ntsc);
        assert_eq!(classify_fields(&[42, 312, 312, 312]), TvStandard::Pal);
        assert_eq!(classify_fields(&[]), TvStandard::Ntsc);
        // A long startup field doesn't sway the median.
        assert_eq!(
            classify_fields(&[45, 285, 282, 262, 262, 262]),
            TvStandard::Ntsc
        );
    }

    #[test]
    fn probe_reads_ntsc_from_a_real_rom() {
        // A real 8 KB (F8) ROM, size-detected: build, run, and count its fields.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../missingno-vcs/tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        assert_eq!(
            probe_tv_standard(&std::fs::read(&path).unwrap(), None),
            TvStandard::Ntsc
        );
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
            if let Some(field) = self.tv.feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            }) {
                self.last_frame = indexed_frame(&field.lines, standard);
                display = Some(VideoFrame::Indexed(self.last_frame.clone()));
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

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(missingno_vcs::board::AUDIO_COUPLING.high_pass())
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Tv {
            standard: self.vcs.tv_standard(),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        frame_interval(self.vcs.tv_standard())
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

/// The core's TIA palette for a standard as the screen path's shared RGB8 slice
/// — NTSC/PAL hue decode, or SECAM's luma-only 8 colours.
fn region_palette(standard: TvStandard) -> std::sync::Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTES: OnceLock<[std::sync::Arc<[RGB8]>; 3]> = OnceLock::new();
    let build = |standard| -> std::sync::Arc<[RGB8]> {
        missingno_vcs::tia::palette(standard)
            .iter()
            .map(|&(r, g, b)| RGB8::new(r, g, b))
            .collect::<Vec<_>>()
            .into()
    };
    let cache = PALETTES.get_or_init(|| {
        [
            build(TvStandard::Ntsc),
            build(TvStandard::Pal),
            build(TvStandard::Secam),
        ]
    });
    let index = match standard {
        TvStandard::Ntsc => 0,
        TvStandard::Pal => 1,
        TvStandard::Secam => 2,
    };
    cache[index].clone()
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

    fn display(&mut self, frame: Option<Frame>) -> Option<VideoFrame> {
        let frame = frame?;
        self.frame_count += 1;
        let standard = self.core.console().tv_standard();
        self.last_frame = indexed_frame(&frame.lines, standard);
        Some(VideoFrame::Indexed(self.last_frame.clone()))
    }
}

impl SystemDebugger for VcsDebugger {
    fn step(&mut self) -> StepOutcome {
        let frame = self.core.step();
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_over(&mut self) -> StepOutcome {
        let (frame, _) = self.core.step_over();
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_frame(&mut self) -> StepOutcome {
        use missingno_vcs::debugger::Stop;
        let (frame, stop) = self.core.step_frame();
        let display = self.display(frame);
        self.refresh();
        match stop {
            Stop::Breakpoint => StepOutcome::Breakpoint { frame: display },
            Stop::BudgetExhausted => StepOutcome::BudgetExhausted,
            Stop::Completed => StepOutcome::Completed { frame: display },
        }
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
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

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(missingno_vcs::board::AUDIO_COUPLING.high_pass())
    }

    fn set_breakpoint(&mut self, address: u32) {
        self.core.set_breakpoint(address as u16);
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.core.clear_breakpoint(address as u16);
    }

    fn breakpoints(&self) -> BTreeSet<u32> {
        self.core.breakpoints().iter().map(|&a| a as u32).collect()
    }

    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        self.core.register_groups()
    }

    fn memory_regions(&self) -> &'static [inspect::MemoryRegion] {
        self.core.memory_regions()
    }

    fn peek(&self, address: u32) -> u8 {
        self.core.peek(address)
    }

    fn pc(&self) -> u32 {
        self.core.pc()
    }

    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        Some(self.core.instruction_set())
    }

    fn family_state(&self) -> &dyn std::any::Any {
        &self.inspect
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Tv {
            standard: self.core.console().tv_standard(),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        let mut state = self.inspect.clone();
        state.frame = frame;
        Box::new(VcsSnapshot::new(state))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.inspect.pc.into(),
            sp: (self.inspect.s as u16 | 0x0100).into(),
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
        frame_interval(self.core.console().tv_standard())
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(VcsConsole {
            vcs: self.core.into_console(),
            title: self.title,
            last_frame: self.last_frame,
            tv: Television::new(VSYNC_LOCK_LINES),
        })
    }
}
