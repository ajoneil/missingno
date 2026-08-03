//! The debugging half of the seam: the VCS under its debugging backend, and
//! the per-frame snapshot the running view renders from. Symbols, code/data
//! logging, and watchpoints have no backend yet — the seam defaults report
//! them absent.

use std::collections::BTreeSet;
use std::time::Duration;

use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::ports::{PanelControl, PeripheralId, PlugError, PortDescriptor, PortId};
use missingno_core::state::{StateRecord, SystemStateSchema};
use missingno_core::system::{
    ControlId, ControlInput, DebugView, FrameOutcome, InspectSnapshot, RunningStatus, StateError,
    StepOutcome, SystemConsole, SystemDebugger,
};
use missingno_core::video::{DisplayTechnology, Frame as VideoFrame, IndexedFrame};

use crate::console::Frame;
use crate::state_schema::vcs_state_schema;
use crate::tv_standard::pixel_aspect;

use super::console::VcsConsole;
use super::controls::{self, apply_control};
use super::frame::{frame_interval, indexed_frame};
use super::inspect::{VcsInspectState, capture};
use super::save_state::{load_state_into, save_state_bytes};
use super::sections::vcs_sidebar_sections;

/// Bytes captured before the program counter, and the total span; the
/// remainder ahead covers the forward disassembly. The 6507 sees a 13-bit bus,
/// but the program counter and these reads wrap in the 16-bit space the peek
/// mirrors into.
const WINDOW_BEHIND: u16 = 128;
const WINDOW_LEN: u16 = 512;

/// A kernel that never completes a frame must not stall a trace capture: bound
/// it at the frame budget's worth of CPU cycles (76 per scanline).
#[cfg(feature = "morepork")]
const CAPTURE_BUDGET_CYCLES: usize = super::frame::FRAME_BUDGET_LINES * 76;

/// The per-frame snapshot for the running view.
pub struct VcsSnapshot {
    pub state: VcsInspectState,
    memory: inspect::MemoryWindow,
    channel_waves: Option<Vec<missingno_core::waveform::ChannelWave>>,
}

impl InspectSnapshot for VcsSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        let s = &self.state;
        crate::debugger::cpu_register_groups(s.pc, s.a, s.x, s.y, s.s, s.p)
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        vcs_sidebar_sections(&self.state)
    }
    fn memory_window(&self) -> Option<&inspect::MemoryWindow> {
        Some(&self.memory)
    }
    fn pc(&self) -> Option<u32> {
        Some(self.state.pc as u32)
    }
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        Some(&missingno_6502::Mos6502)
    }
    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.channel_waves.clone()
    }
}

pub(super) struct VcsDebugger {
    core: crate::debugger::Debugger,
    title: String,
    rom_sha256: String,
    last_frame: IndexedFrame,
    inspect: VcsInspectState,
    frame_count: u64,
}

impl VcsDebugger {
    pub(super) fn new(
        core: crate::debugger::Debugger,
        title: String,
        rom_sha256: String,
        last_frame: IndexedFrame,
    ) -> Self {
        let mut this = VcsDebugger {
            core,
            title,
            rom_sha256,
            last_frame,
            inspect: VcsInspectState::default(),
            frame_count: 0,
        };
        this.refresh();
        this
    }

    /// Rebuild the inspection state from the console (peek-only).
    fn refresh(&mut self) {
        self.inspect = capture(self.core.console(), self.frame_count);
    }

    fn display(&mut self, frame: Option<Frame>) -> Option<VideoFrame> {
        let frame = frame?;
        self.frame_count += 1;
        let standard = self.core.console().tv_standard();
        self.last_frame = indexed_frame(&frame.lines, standard);
        Some(VideoFrame::Indexed(self.last_frame.clone()))
    }
}

impl SystemConsole for VcsDebugger {
    /// One frame under the debugger: the breakpoints and watches still stop it,
    /// and the host learns why only through [`SystemDebugger::run_frame`].
    fn step_frame(&mut self) -> FrameOutcome {
        FrameOutcome {
            display: SystemDebugger::run_frame(self).into_frame(),
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.core.console_mut().power_cycle();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(self.core.console_mut(), control, input);
    }

    fn panel_controls(&self) -> &'static [PanelControl] {
        controls::PANEL_CONTROLS
    }

    fn ports(&self) -> &'static [PortDescriptor] {
        controls::PORTS
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        controls::plugged(self.core.console(), port)
    }

    fn plug(&mut self, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        controls::plug(self.core.console_mut(), port, peripheral)
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.core.console_mut().drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(crate::board::AUDIO_COUPLING.high_pass())
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: self.core.console().tv_standard(),
            pixel_aspect: pixel_aspect(self.core.console().tv_standard()),
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        frame_interval(self.core.console().tv_standard())
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        Some(vcs_state_schema())
    }

    fn read_state(&self) -> Option<StateRecord> {
        Some(crate::snapshot::read_state(self.core.console()))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(self.core.console(), &self.last_frame, &self.rom_sha256)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let result = load_state_into(self.core.console_mut(), bytes, &self.rom_sha256);
        if result.is_ok() {
            self.refresh();
        }
        result
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        self
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

    fn run_frame(&mut self) -> StepOutcome {
        use crate::debugger::Stop;
        let (frame, stop) = self.core.step_frame();
        let display = self.display(frame);
        self.refresh();
        match stop {
            Stop::Breakpoint => StepOutcome::Breakpoint { frame: display },
            Stop::Watch => match self.core.last_watch_hit() {
                Some(watch) => StepOutcome::WatchHit(watch),
                None => StepOutcome::Breakpoint { frame: display },
            },
            Stop::BudgetExhausted => StepOutcome::BudgetExhausted,
            Stop::Completed => StepOutcome::Completed { frame: display },
        }
    }

    fn tick_name(&self) -> Option<&'static str> {
        Some("colour clock")
    }

    fn step_tick(&mut self) {
        self.core.console_mut().step_clock();
        self.refresh();
    }

    fn set_wave_capture(&mut self, on: bool) {
        self.core.console_mut().set_wave_capture(on);
    }

    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.core.console().channel_waves()
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

    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        vcs_sidebar_sections(&self.inspect)
    }

    fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
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

    fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        self.core.present_address(address)
    }

    fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        self.core.locate_bank_window(bank, window)
    }

    fn watchables(&self) -> &'static [inspect::Watchable] {
        self.core.watchables()
    }

    fn add_watch(&mut self, watch: inspect::Watch) {
        self.core.add_watch(watch);
    }

    fn remove_watch(&mut self, watch: &inspect::Watch) {
        self.core.remove_watch(watch);
    }

    fn watches(&self) -> Vec<inspect::Watch> {
        self.core.watches()
    }

    fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.core.last_watch_hit()
    }

    fn family_state(&self) -> &dyn std::any::Any {
        &self.inspect
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        let mut state = self.inspect.clone();
        state.frame = frame;
        let base = state.pc.wrapping_sub(WINDOW_BEHIND);
        let bytes = (0..WINDOW_LEN)
            .map(|i| self.core.peek(base.wrapping_add(i) as u32))
            .collect();
        Box::new(VcsSnapshot {
            state,
            memory: inspect::MemoryWindow {
                base: base as u32,
                bytes,
            },
            channel_waves: self.core.console().channel_waves(),
        })
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

    fn capture_trace(&mut self, path: &std::path::Path) -> Option<VideoFrame> {
        #[cfg(feature = "morepork")]
        {
            use crate::trace::{TraceScope, Tracer, Trigger};

            let mut tracer = Tracer::create_hashed(
                path,
                self.rom_sha256.clone(),
                self.core.console().tv_standard(),
                Trigger::Cycle,
                TraceScope::Full,
            )
            .ok()?;
            let vcs = self.core.console_mut();
            let mut cycles = 0u16;
            let mut frame = None;
            for _ in 0..CAPTURE_BUDGET_CYCLES {
                tracer.capture(vcs, cycles).ok()?;
                vcs.step_cpu_cycle();
                cycles = 1;
                if let Some(completed) = vcs.take_frame() {
                    frame = Some(completed);
                    break;
                }
            }
            let frame = frame?;
            tracer.mark_frame(Some(&frame)).ok()?;
            tracer.finish().ok()?;
            self.display(Some(frame))
        }
        #[cfg(not(feature = "morepork"))]
        {
            let _ = path;
            None
        }
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(VcsConsole::new(
            self.core.into_console(),
            self.title,
            self.rom_sha256,
            self.last_frame,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Vcs;
    use crate::debug::frame::blank_frame;
    use crate::{DumpFit, TvStandard};

    fn f8_test_rom() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        std::fs::read(&path).unwrap()
    }

    #[test]
    fn snapshot_register_groups_match_live() {
        let mut vcs = Vcs::new(&f8_test_rom(), TvStandard::Ntsc, None, DumpFit::Exact).unwrap();
        for _ in 0..64 {
            vcs.step_instruction();
        }
        let live = crate::debugger::Debugger::new(vcs);
        let cpu = &live.console().cpu;
        let snapshot = VcsSnapshot {
            state: VcsInspectState {
                a: cpu.a,
                x: cpu.x,
                y: cpu.y,
                s: cpu.s,
                p: cpu.p,
                pc: cpu.pc,
                ..Default::default()
            },
            memory: inspect::MemoryWindow {
                base: 0,
                bytes: Vec::new(),
            },
            channel_waves: None,
        };
        assert_eq!(
            format!("{:?}", live.register_groups()),
            format!("{:?}", snapshot.register_groups())
        );
    }

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let vcs = Vcs::new(&f8_test_rom(), TvStandard::Ntsc, None, DumpFit::Exact).unwrap();
        let mut debugger = VcsDebugger::new(
            crate::debugger::Debugger::new(vcs),
            "test".to_string(),
            String::new(),
            blank_frame(),
        );
        for _ in 0..64 {
            debugger.step();
        }
        let live = SystemDebugger::sidebar_sections(&debugger);
        let snapshot = debugger.snapshot(0);
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.sidebar_sections())
        );
    }
}
