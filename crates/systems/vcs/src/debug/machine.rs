//! The VCS's binding to the machine seam: the core the hooks drive — the
//! console under its debugging backend, and the television whose VSYNC
//! integrator decides where one field ends — and the hooks themselves. Symbols
//! and code/data logging have no backend yet; the seam defaults report them
//! absent.

use std::time::Duration;

use missingno_core::inspect::{
    AddressDisplay, MemoryRegion, RegisterGroup, Section, Watch, Watchable,
};
use missingno_core::isa::InstructionSet;
use missingno_core::launch::{LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind};
use missingno_core::machine::{
    BoundaryState, CoreRun, CoreStop, Machine, MachineConsole, StateIdentity, StopSet,
};
use missingno_core::ports::{PanelControl, PeripheralId, PlugError, PortDescriptor, PortId};
use missingno_core::state::{PixelFormat, StateRecord, SystemStateSchema};
use missingno_core::state_file::StateFrame;
use missingno_core::system::{
    ControlId, ControlInput, DebugView, InspectSnapshot, RunningStatus, StateError, SystemConsole,
};
use missingno_core::video::{
    self, DisplayTechnology, Field, Frame as VideoFrame, IndexedFrame, Television,
};
use missingno_core::waveform::ChannelWave;

use crate::TvStandard;
use crate::cartridge::{CartType, CartridgeError, DumpFit};
use crate::console::Vcs;
use crate::debugger::{Debugger, Stop, Stops};
use crate::state_schema::vcs_state_schema;
use crate::tia::{Scanline, VISIBLE_CLOCKS};
use crate::tv_standard::pixel_aspect;

use super::controls::{self, apply_control};
use super::frame::{
    FRAME_BUDGET_LINES, VSYNC_LOCK_LINES, blank_frame, frame_interval, indexed_frame,
};
use super::inspect::{VcsInspectState, capture};
use super::probe::probe_tv_standard;
use super::sections::vcs_sidebar_sections;

/// A kernel that never completes a frame must not stall a trace capture: bound
/// it at the frame budget's worth of CPU cycles (76 per scanline).
#[cfg(feature = "morepork")]
const CAPTURE_BUDGET_CYCLES: usize = FRAME_BUDGET_LINES * 76;

/// The broadcast standard the console's video is decoded for.
pub const TV_STANDARD: &str = "tv-standard";
/// The board the cartridge's silicon sits on.
pub const BOARD: &str = "board";
/// The dump runs past the cartridge's silicon.
pub const OVERDUMP: &str = "overdump";

/// The options the VCS accepts at launch. A cart carries no header, so what a
/// catalogue says about its region, its board and its dump is all a loader has —
/// the media itself settles nothing.
pub fn launch_options(_rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
    vec![
        LaunchOptionDescriptor {
            id: TV_STANDARD,
            label: "TV standard",
            kind: LaunchOptionKind::Choice {
                choices: TvStandard::all()
                    .into_iter()
                    .map(|standard| LaunchChoice {
                        value: standard.code(),
                        label: standard.name(),
                    })
                    .collect(),
            },
        },
        LaunchOptionDescriptor {
            id: BOARD,
            label: "Cartridge board",
            kind: LaunchOptionKind::Choice {
                choices: CartType::all()
                    .filter(|board| board.built())
                    .map(|board| LaunchChoice {
                        value: board.code(),
                        label: board.display_name(),
                    })
                    .collect(),
            },
        },
        LaunchOptionDescriptor {
            id: OVERDUMP,
            label: "Overdump",
            kind: LaunchOptionKind::Toggle,
        },
    ]
}

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
    overdump: bool,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    // The library's metadata is authoritative; carts carry no region header and
    // the size heuristic can't always name the board, so fall back only when
    // the game-db is silent — then probe the standard from the ROM's own field
    // length. Pacing, aspect, and palette follow the standard.
    let cart = cart_type.and_then(CartType::from_code);
    let fit = match overdump {
        true => DumpFit::Overdump,
        false => DumpFit::Exact,
    };
    let region = match tv_standard {
        Some(standard) => standard,
        None => probe_tv_standard(rom, cart, fit),
    };
    let fingerprint = rom_fingerprint(rom);
    let core = VcsCore::new(Vcs::new(rom, region, cart, fit)?, fingerprint);
    Ok(Box::new(
        MachineConsole::<VcsSystem>::new(core, title).with_identity(StateIdentity {
            rom_fingerprint: fingerprint,
        }),
    ))
}

/// SHA-256 of the raw ROM image, taken at load (the cartridge does not retain a
/// plain board's image), so a save state can refuse a ROM it was not written for.
fn rom_fingerprint(rom: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(rom).into()
}

/// What the seam drives: the console under its debugging backend, the
/// television it is watched on, and the ROM binding a trace and a save state
/// carry.
pub struct VcsCore {
    debugger: Debugger,
    tv: Television<VISIBLE_CLOCKS>,
    /// The field the television has locked, waiting for the seam to take it.
    locked: Option<Field<VISIBLE_CLOCKS>>,
    /// The last field shown, as a save state's framebuffer carries it.
    last_display: IndexedFrame,
    /// The ROM image's digest, which a captured trace binds itself to.
    #[cfg_attr(not(feature = "morepork"), allow(dead_code))]
    rom_sha256: [u8; 32],
}

impl VcsCore {
    fn new(vcs: Vcs, rom_sha256: [u8; 32]) -> Self {
        VcsCore {
            debugger: Debugger::new(vcs),
            tv: Television::new(VSYNC_LOCK_LINES),
            locked: None,
            last_display: blank_frame(),
            rom_sha256,
        }
    }

    fn vcs(&self) -> &Vcs {
        self.debugger.console()
    }

    fn vcs_mut(&mut self) -> &mut Vcs {
        self.debugger.console_mut()
    }

    /// Show one completed scanline; the television says when the field ends.
    fn feed(&mut self, line: Scanline) -> Option<Field<VISIBLE_CLOCKS>> {
        self.tv.feed(video::Scanline {
            pixels: line.pixels,
            vsync: line.vsync,
        })
    }

    /// Show every scanline the console has completed since the last look — the
    /// path for a run that advances by instructions rather than whole lines.
    fn absorb_lines(&mut self) {
        while let Some(line) = self.vcs_mut().take_line() {
            if let Some(field) = self.feed(line) {
                self.locked = Some(field);
            }
        }
    }

    /// The field the television has locked since the last take.
    fn take_field(&mut self) -> Option<Field<VISIBLE_CLOCKS>> {
        self.absorb_lines();
        self.locked.take()
    }

    /// A completed field in the picture window and palette its standard implies.
    fn present(&mut self, field: &Field<VISIBLE_CLOCKS>) -> IndexedFrame {
        self.last_display = indexed_frame(&field.lines, self.vcs().tv_standard());
        self.last_display.clone()
    }

    /// Advance one instruction and show what it drew, reporting the stop it
    /// landed on.
    fn step_instruction(&mut self, stops: &Stops) -> (Option<Stop>, Option<IndexedFrame>) {
        let stop = self.debugger.step(stops);
        let frame = self.take_field().map(|field| self.present(&field));
        (stop, frame)
    }
}

/// The per-frame snapshot for the running view; the seam's wrapper carries the
/// memory window, the instruction set, and the captured waveforms around it.
pub struct VcsSnapshot {
    pub state: VcsInspectState,
}

impl InspectSnapshot for VcsSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        VcsSystem::register_groups(&self.state)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        vcs_sidebar_sections(&self.state)
    }
}

pub struct VcsSystem;

impl Machine for VcsSystem {
    type Core = VcsCore;
    type Frame = IndexedFrame;
    type InspectState = VcsInspectState;

    /// The nominal NTSC field; a console wired to another standard paces from
    /// its own.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    /// Bounds a syncless kernel: ~20 NTSC frames of minimum-length instructions.
    const RUN_BUDGET: u32 = 200_000;

    fn frame_interval(core: &VcsCore) -> Duration {
        frame_interval(core.vcs().tv_standard())
    }

    fn pc(core: &VcsCore) -> u16 {
        core.vcs().cpu.pc
    }

    fn peek(core: &VcsCore, address: u16) -> u8 {
        core.vcs().peek(address)
    }

    fn peek_region(core: &VcsCore, address: u32) -> u8 {
        core.debugger.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn InstructionSet> {
        Some(&missingno_mos_6502::Mos6502)
    }

    fn step_instruction(core: &mut VcsCore) {
        core.vcs_mut().step_instruction();
    }

    fn take_frame(core: &mut VcsCore) -> Option<IndexedFrame> {
        core.take_field().map(|field| core.present(&field))
    }

    /// Drive the console scanline by scanline through the television. Bounded so
    /// a kernel that never syncs cannot stall the emulation thread.
    fn step_frame(core: &mut VcsCore) -> Option<IndexedFrame> {
        for _ in 0..FRAME_BUDGET_LINES {
            let line = core.vcs_mut().step_scanline();
            if let Some(field) = core.feed(line) {
                return Some(core.present(&field));
            }
        }
        None
    }

    fn power_cycle(core: &mut VcsCore) {
        core.vcs_mut().power_cycle();
    }

    fn apply_control(core: &mut VcsCore, control: ControlId, input: ControlInput) {
        apply_control(core.vcs_mut(), control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        controls::PORTS
    }

    fn plugged(core: &VcsCore, port: PortId) -> Option<PeripheralId> {
        controls::plugged(core.vcs(), port)
    }

    fn plug(core: &mut VcsCore, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        controls::plug(core.vcs_mut(), port, peripheral)
    }

    fn panel_controls() -> &'static [PanelControl] {
        controls::PANEL_CONTROLS
    }

    fn drain_audio_samples(core: &mut VcsCore) -> Vec<(f32, f32)> {
        core.vcs_mut().drain_audio_samples()
    }

    fn audio_coupling() -> Option<missingno_core::HighPass> {
        Some(crate::board::AUDIO_COUPLING.high_pass())
    }

    fn video_out(core: &VcsCore) -> DisplayTechnology {
        let standard = core.vcs().tv_standard();
        DisplayTechnology::Crt {
            standard,
            pixel_aspect: pixel_aspect(standard),
        }
    }

    fn display_frame(frame: &IndexedFrame) -> VideoFrame {
        VideoFrame::Indexed(frame.clone())
    }

    fn blank_display() -> VideoFrame {
        VideoFrame::Indexed(blank_frame())
    }

    fn state_schema() -> Option<&'static SystemStateSchema> {
        Some(vcs_state_schema())
    }

    fn read_state(core: &VcsCore) -> Option<StateRecord> {
        Some(crate::snapshot::read_state(core.vcs()))
    }

    /// A save is only faithful at an instruction boundary, where the CPU carries
    /// no micro-sequencer residue.
    fn capture_boundary(core: &VcsCore) -> Result<BoundaryState, StateError> {
        if !core.vcs().at_instruction_boundary() {
            return Err(StateError::Unsupported);
        }
        Ok(BoundaryState {
            record: crate::snapshot::read_state(core.vcs()),
            memory: crate::snapshot::capture_memory(core.vcs()),
            frame: Some(state_frame(&core.last_display)),
        })
    }

    fn restore_boundary(
        core: &mut VcsCore,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
        _frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        crate::snapshot::restore(core.vcs_mut(), record, memory)
    }

    fn step_over_target(core: &VcsCore) -> Option<u16> {
        core.debugger.step_over_target()
    }

    fn tick_name() -> Option<&'static str> {
        Some("colour clock")
    }

    fn step_tick(core: &mut VcsCore) {
        core.vcs_mut().step_clock();
        core.absorb_lines();
    }

    /// Run until the television locks a field, a breakpoint, or a watch. A stop
    /// takes precedence: when the instruction that completes the field also
    /// lands on one, the field rides out with the stop, so the pending pc is
    /// reported now rather than stepped past on the next call.
    fn run_frame(core: &mut VcsCore, stops: &StopSet) -> CoreRun<IndexedFrame> {
        let stops = Stops::new(stops);
        for _ in 0..Self::RUN_BUDGET {
            let (stop, frame) = core.step_instruction(&stops);
            match stop {
                Some(Stop::Breakpoint) => {
                    return CoreRun {
                        stop: CoreStop::Breakpoint,
                        frame,
                    };
                }
                Some(Stop::Watch(watch)) => {
                    return CoreRun {
                        stop: CoreStop::WatchHit(watch),
                        frame,
                    };
                }
                None if frame.is_some() => {
                    return CoreRun {
                        stop: CoreStop::Completed,
                        frame,
                    };
                }
                None => {}
            }
        }
        CoreRun {
            stop: CoreStop::BudgetExhausted,
            frame: None,
        }
    }

    /// Run to the address the call returns to, carrying out the newest field
    /// completed on the way.
    fn run_step_over(
        core: &mut VcsCore,
        stops: &StopSet,
        return_address: u16,
    ) -> CoreRun<IndexedFrame> {
        let stops = Stops::new(stops);
        let mut frame = None;
        for _ in 0..Self::RUN_BUDGET {
            let (stop, completed) = core.step_instruction(&stops);
            frame = completed.or(frame);
            if core.debugger.at_address(return_address) {
                return CoreRun {
                    stop: CoreStop::Completed,
                    frame,
                };
            }
            match stop {
                Some(Stop::Breakpoint) => {
                    return CoreRun {
                        stop: CoreStop::Breakpoint,
                        frame,
                    };
                }
                Some(Stop::Watch(watch)) => {
                    return CoreRun {
                        stop: CoreStop::WatchHit(watch),
                        frame,
                    };
                }
                None => {}
            }
        }
        CoreRun {
            stop: CoreStop::BudgetExhausted,
            frame,
        }
    }

    fn memory_regions(core: &VcsCore) -> Vec<MemoryRegion> {
        core.debugger.memory_regions()
    }

    fn present_address(core: &VcsCore, address: u32) -> AddressDisplay {
        core.debugger.present_address(address)
    }

    fn locate_bank_window(core: &VcsCore, bank: u16, window: u32) -> Option<u32> {
        core.debugger.locate_bank_window(bank, window)
    }

    fn watchables() -> &'static [Watchable] {
        crate::debugger::watchables()
    }

    fn watch_supported(watch: &Watch) -> bool {
        crate::debugger::supports_watch(watch)
    }

    fn set_wave_capture(core: &mut VcsCore, on: bool) {
        core.vcs_mut().set_wave_capture(on);
    }

    fn channel_waves(core: &VcsCore) -> Option<Vec<ChannelWave>> {
        core.vcs().channel_waves()
    }

    fn capture_trace(core: &mut VcsCore, path: &std::path::Path) -> Option<VideoFrame> {
        #[cfg(feature = "morepork")]
        {
            use crate::trace::{TraceScope, Tracer, Trigger};

            let mut tracer = Tracer::create_hashed(
                path,
                hex_digest(&core.rom_sha256),
                core.vcs().tv_standard(),
                Trigger::Cycle,
                TraceScope::Full,
            )
            .ok()?;
            let mut cycles = 0u16;
            let mut field = None;
            for _ in 0..CAPTURE_BUDGET_CYCLES {
                tracer.capture(core.vcs(), cycles).ok()?;
                core.vcs_mut().step_cpu_cycle();
                cycles = 1;
                if let Some(completed) = core.take_field() {
                    field = Some(completed);
                    break;
                }
            }
            let field = field?;
            let frame = crate::console::Frame {
                lines: field.lines.clone(),
            };
            tracer.mark_frame(Some(&frame)).ok()?;
            tracer.finish().ok()?;
            Some(VideoFrame::Indexed(core.present(&field)))
        }
        #[cfg(not(feature = "morepork"))]
        {
            let _ = (core, path);
            None
        }
    }

    fn inspect(core: &VcsCore, frame_count: u64) -> VcsInspectState {
        capture(core.vcs(), frame_count)
    }

    fn register_groups(state: &VcsInspectState) -> Vec<RegisterGroup> {
        crate::debugger::cpu_register_groups(state.pc, state.a, state.x, state.y, state.s, state.p)
    }

    fn sidebar_sections(state: &VcsInspectState) -> Vec<Section> {
        vcs_sidebar_sections(state)
    }

    fn snapshot(state: &VcsInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(VcsSnapshot { state })
    }

    fn running_status(state: &VcsInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc.into(),
            sp: (state.s as u16 | 0x0100).into(),
            video_label: "TIA",
            video_summary: format!("beam {} · line {}", state.beam, state.scanline),
            frame,
        }
    }
}

/// The displayed field as a save-state framebuffer blob — informational; a
/// restored console regenerates its display from the restored hardware.
fn state_frame(frame: &IndexedFrame) -> StateFrame {
    StateFrame {
        width: frame.width,
        height: Some(frame.height),
        format: PixelFormat::Indexed8,
        data: frame.pixels.to_vec(),
    }
}

/// The hex spelling of the ROM digest, as a trace's media binding carries it.
#[cfg(feature = "morepork")]
fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::system::{StepOutcome, SystemDebugger};

    use crate::debugger::test_support::reset_to_f000;

    fn f8_test_rom() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        std::fs::read(&path).unwrap()
    }

    fn debugger(rom: &[u8]) -> Box<dyn SystemDebugger> {
        create_console(rom, "test".into(), Some(TvStandard::Ntsc), None, false)
            .expect("console builds")
            .into_debugger()
    }

    #[test]
    fn video_out_reports_a_crt_with_the_carts_standard() {
        // A 4 KiB ROM whose reset vector points at its origin; the caller-
        // supplied standard maps straight onto the CRT descriptor.
        let mut rom = vec![0xEA; 0x1000];
        reset_to_f000(&mut rom);
        for standard in [TvStandard::Ntsc, TvStandard::Pal, TvStandard::Secam] {
            let console = create_console(&rom, "test".into(), Some(standard), None, false)
                .expect("console builds");
            match console.video_out() {
                DisplayTechnology::Crt {
                    standard: reported,
                    pixel_aspect,
                } => {
                    assert_eq!(reported, standard);
                    assert_eq!(pixel_aspect, crate::tv_standard::pixel_aspect(standard));
                }
                other => panic!("VCS should drive a CRT, got {other:?}"),
            }
            assert_eq!(console.frame_interval(), frame_interval(standard));
        }
    }

    #[test]
    fn snapshot_register_groups_match_live() {
        let mut debugger = debugger(&f8_test_rom());
        for _ in 0..64 {
            debugger.step();
        }
        assert_eq!(
            format!("{:?}", debugger.register_groups()),
            format!("{:?}", debugger.snapshot(0).register_groups())
        );
    }

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let mut debugger = debugger(&f8_test_rom());
        for _ in 0..64 {
            debugger.step();
        }
        let live = SystemDebugger::sidebar_sections(&*debugger);
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", debugger.snapshot(0).sidebar_sections())
        );
    }

    /// A kernel that lands the field-completing instruction on the loop's first
    /// instruction: two WSYNCs build picture lines, VSYNC is asserted, and two
    /// more WSYNCs halt through the sync lines the television integrates, so the
    /// field locks exactly as the CPU resumes into the loop. The NOP at `$F00C`
    /// spans that wrap and leaves the pc at the `JMP` at `$F00D`.
    fn field_completes_at_loop_rom() -> Vec<u8> {
        let mut bank = vec![0u8; 0x1000];
        bank[0x000..0x010].copy_from_slice(&[
            0x85, 0x02, // STA WSYNC
            0x85, 0x02, // STA WSYNC
            0xA9, 0x02, // LDA #2
            0x85, 0x00, // STA VSYNC
            0x85, 0x02, // STA WSYNC
            0x85, 0x02, // STA WSYNC
            0xEA, // NOP        ($F00C)
            0x4C, 0x0D, 0xF0, // JMP $F00D  ($F00D)
        ]);
        reset_to_f000(&mut bank);
        bank
    }

    #[test]
    fn run_frame_reports_a_stop_coincident_with_the_field() {
        // A breakpoint on the pc the field-completing instruction lands on must
        // be reported on the first call, carrying the completed field — not
        // masked by it and stepped past on the next call.
        let mut debugger = debugger(&field_completes_at_loop_rom());
        debugger.set_breakpoint(0xF00D);
        match debugger.run_frame() {
            StepOutcome::Breakpoint { frame } => {
                assert!(
                    frame.is_some(),
                    "the completed field rides out with the stop"
                )
            }
            _ => panic!("expected a breakpoint stop"),
        }
        assert_eq!(debugger.pc() & 0x1FFF, 0xF00D & 0x1FFF);
    }
}
