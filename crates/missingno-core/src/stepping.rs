//! A shared seam adapter for families whose debugger is plain instruction
//! stepping over the core: PC breakpoints, one typed inspection state
//! refreshed after every step, and indexed frames. A family implements
//! [`SteppingSystem`] as a flat list of hooks; [`SteppingConsole`] and
//! [`SteppingDebugger`] carry the seam's control flow once.

use std::any::Any;
use std::collections::BTreeSet;
use std::time::Duration;

use crate::TvStandard;
use crate::inspect::{MemoryWindow, RegisterGroup, Section};
use crate::isa::InstructionSet;
use crate::system::{
    ControlId, ControlInput, DebugView, FrameOutcome, InspectSnapshot, RunningStatus, StepOutcome,
    SystemConsole, SystemDebugger,
};
use crate::video::{DisplayTechnology, Frame, IndexedFrame};

/// Bytes captured before the program counter — enough for the disassembly's
/// backward sweep — and the total span, its remainder covering the forward
/// window. Both fit inside the 16-bit address space these families wrap in.
const WINDOW_BEHIND: u16 = 128;
const WINDOW_LEN: u16 = 512;

pub trait SteppingSystem: 'static {
    type Core: Send + 'static;
    type Frame;
    type InspectState: Clone + Send + 'static;

    /// Wall-clock duration of one emulated frame, for the pacing loop.
    const FRAME_INTERVAL: Duration;
    /// Instruction budget for one debugger-driven frame or step-over, so a
    /// core that never completes a frame cannot stall the UI.
    const RUN_BUDGET: u32;

    /// Display aspect of one pixel — the constant this system's indexed frames
    /// carry. These families raster NTSC-timed frames.
    const PIXEL_ASPECT: f32;
    fn pc(core: &Self::Core) -> u16;
    /// Side-effect-free read of the CPU address space, for the memory viewer
    /// and the disassembly.
    fn peek(core: &Self::Core, address: u16) -> u8;
    /// The decode-for-display front end, when the family has one. `None`
    /// leaves the disassembly to fall back to raw bytes.
    fn instruction_set() -> Option<&'static dyn InstructionSet> {
        None
    }
    fn step_instruction(core: &mut Self::Core);
    /// The frame completed since the last take, if any.
    fn take_frame(core: &mut Self::Core) -> Option<Self::Frame>;
    /// Run up to one frame on the console's own budget.
    fn step_frame(core: &mut Self::Core) -> Option<Self::Frame>;
    fn power_cycle(core: &mut Self::Core);
    fn apply_control(core: &mut Self::Core, control: ControlId, input: ControlInput);
    fn drain_audio_samples(core: &mut Self::Core) -> Vec<(f32, f32)>;
    fn indexed_frame(frame: &Self::Frame) -> IndexedFrame;
    /// What the display shows before the first frame completes.
    fn blank_frame() -> IndexedFrame;
    /// The return address to run to when the instruction at PC is a call;
    /// `None` steps normally.
    fn step_over_target(core: &Self::Core) -> Option<u16>;
    /// Rebuild the typed inspection state from the core (peek-only).
    fn inspect(core: &Self::Core, frame_count: u64) -> Self::InspectState;
    /// The register groups this system exposes for the schema-driven view.
    fn register_groups(_state: &Self::InspectState) -> Vec<RegisterGroup> {
        Vec::new()
    }
    /// The structured sidebar sections this system exposes, built from the
    /// typed state so the live and running views agree. Defaults to a single
    /// CPU section from the register groups; a system overrides to add its
    /// video section.
    fn sidebar_sections(state: &Self::InspectState) -> Vec<Section> {
        crate::inspect::default_sections(Self::register_groups(state))
    }
    /// An owned snapshot of the state, stamped with the UI's frame counter.
    fn snapshot(state: &Self::InspectState, frame: u64) -> DebugView;
    fn running_status(state: &Self::InspectState, frame: u64) -> RunningStatus;
}

pub struct SteppingConsole<S: SteppingSystem> {
    core: S::Core,
    title: String,
    last_frame: IndexedFrame,
}

impl<S: SteppingSystem> SteppingConsole<S> {
    pub fn new(core: S::Core, title: String) -> Self {
        SteppingConsole {
            core,
            title,
            last_frame: S::blank_frame(),
        }
    }
}

impl<S: SteppingSystem> SystemConsole for SteppingConsole<S> {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = S::step_frame(&mut self.core).map(|frame| {
            self.last_frame = S::indexed_frame(&frame);
            Frame::Indexed(self.last_frame.clone())
        });
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        S::power_cycle(&mut self.core);
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        S::apply_control(&mut self.core, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        S::drain_audio_samples(&mut self.core)
    }

    fn screen_display(&self) -> Frame {
        Frame::Indexed(self.last_frame.clone())
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: TvStandard::Ntsc,
            pixel_aspect: S::PIXEL_ASPECT,
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        S::FRAME_INTERVAL
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        Box::new(SteppingDebugger::<S>::new(
            self.core,
            self.title,
            self.last_frame,
        ))
    }
}

/// A stepping core under the seam's debugger. Symbols, code/data logging,
/// and watchpoints have no backend — the seam defaults report them absent.
pub struct SteppingDebugger<S: SteppingSystem> {
    core: S::Core,
    breakpoints: BTreeSet<u16>,
    title: String,
    last_frame: IndexedFrame,
    inspect: S::InspectState,
    frame_count: u64,
}

impl<S: SteppingSystem> SteppingDebugger<S> {
    fn new(core: S::Core, title: String, last_frame: IndexedFrame) -> Self {
        let inspect = S::inspect(&core, 0);
        SteppingDebugger {
            core,
            breakpoints: BTreeSet::new(),
            title,
            last_frame,
            inspect,
            frame_count: 0,
        }
    }

    fn refresh(&mut self) {
        self.inspect = S::inspect(&self.core, self.frame_count);
    }

    fn display(&mut self, frame: Option<S::Frame>) -> Option<Frame> {
        let frame = frame?;
        self.frame_count += 1;
        self.last_frame = S::indexed_frame(&frame);
        Some(Frame::Indexed(self.last_frame.clone()))
    }

    fn at_breakpoint(&self) -> bool {
        self.breakpoints.contains(&S::pc(&self.core))
    }
}

impl<S: SteppingSystem> SystemDebugger for SteppingDebugger<S> {
    fn step(&mut self) -> StepOutcome {
        S::step_instruction(&mut self.core);
        let frame = S::take_frame(&mut self.core);
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_over(&mut self) -> StepOutcome {
        let Some(return_address) = S::step_over_target(&self.core) else {
            return self.step();
        };
        let mut frame = None;
        for _ in 0..S::RUN_BUDGET {
            S::step_instruction(&mut self.core);
            frame = S::take_frame(&mut self.core).or(frame);
            if S::pc(&self.core) == return_address || self.at_breakpoint() {
                break;
            }
        }
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_frame(&mut self) -> StepOutcome {
        let mut breakpoint_hit = false;
        let mut frame = None;
        for _ in 0..S::RUN_BUDGET {
            S::step_instruction(&mut self.core);
            if let Some(finished) = S::take_frame(&mut self.core) {
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
        if breakpoint_hit {
            StepOutcome::Breakpoint { frame: display }
        } else {
            StepOutcome::Completed { frame: display }
        }
    }

    fn screen_display(&self) -> Frame {
        Frame::Indexed(self.last_frame.clone())
    }

    fn reset(&mut self) {
        S::power_cycle(&mut self.core);
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        S::apply_control(&mut self.core, control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        S::drain_audio_samples(&mut self.core)
    }

    fn set_breakpoint(&mut self, address: u32) {
        self.breakpoints.insert(address as u16);
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.breakpoints.remove(&(address as u16));
    }

    fn breakpoints(&self) -> BTreeSet<u32> {
        self.breakpoints.iter().map(|&a| a as u32).collect()
    }

    fn peek(&self, address: u32) -> u8 {
        S::peek(&self.core, address as u16)
    }

    fn pc(&self) -> u32 {
        S::pc(&self.core) as u32
    }

    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        S::instruction_set()
    }

    fn family_state(&self) -> &dyn std::any::Any {
        &self.inspect
    }

    fn register_groups(&self) -> Vec<RegisterGroup> {
        S::register_groups(&self.inspect)
    }

    fn sidebar_sections(&self) -> Vec<Section> {
        S::sidebar_sections(&self.inspect)
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: TvStandard::Ntsc,
            pixel_aspect: S::PIXEL_ASPECT,
        }
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        let pc = S::pc(&self.core);
        let base = pc.wrapping_sub(WINDOW_BEHIND);
        let bytes = (0..WINDOW_LEN)
            .map(|i| S::peek(&self.core, base.wrapping_add(i)))
            .collect();
        Box::new(SteppingSnapshot {
            inner: S::snapshot(&self.inspect, frame),
            pc,
            memory: MemoryWindow {
                base: base as u32,
                bytes,
            },
            instruction_set: S::instruction_set(),
        })
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        S::running_status(&self.inspect, frame)
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        S::FRAME_INTERVAL
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(SteppingConsole::<S> {
            core: self.core,
            title: self.title,
            last_frame: self.last_frame,
        })
    }
}

/// Wraps a family's per-frame snapshot with the shared running-view fuel the
/// stepping seam captures generically: the program counter, a PC-anchored
/// memory window, and the instruction set. The family's own state stays
/// reachable through `family_state` for its typed panes.
struct SteppingSnapshot {
    inner: DebugView,
    pc: u16,
    memory: MemoryWindow,
    instruction_set: Option<&'static dyn InstructionSet>,
}

impl InspectSnapshot for SteppingSnapshot {
    fn frame(&self) -> u64 {
        self.inner.frame()
    }
    fn family_state(&self) -> &dyn Any {
        self.inner.family_state()
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        self.inner.register_groups()
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        self.inner.sidebar_sections()
    }
    fn memory_window(&self) -> Option<&MemoryWindow> {
        Some(&self.memory)
    }
    fn pc(&self) -> Option<u32> {
        Some(self.pc as u32)
    }
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        self.instruction_set
    }
}
