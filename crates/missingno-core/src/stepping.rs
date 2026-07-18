//! A shared seam adapter for families whose debugger is plain instruction
//! stepping over the core: PC breakpoints, one typed inspection state
//! refreshed after every step, and indexed frames. A family implements
//! [`SteppingSystem`] as a flat list of hooks; [`SteppingConsole`] and
//! [`SteppingDebugger`] carry the seam's control flow once.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::TvStandard;
use crate::system::{
    ControlId, ControlInput, DebugView, FrameOutcome, RunningStatus, StepOutcome, SystemConsole,
    SystemDebugger,
};
use crate::video::{Frame, IndexedFrame, VideoOut};

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

    fn video_out(&self) -> VideoOut {
        VideoOut::Tv {
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

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Ok(Box::new(SteppingDebugger::<S>::new(
            self.core,
            self.title,
            self.last_frame,
        )))
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn family_state(&self) -> &dyn std::any::Any {
        &self.inspect
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Tv {
            standard: TvStandard::Ntsc,
            pixel_aspect: S::PIXEL_ASPECT,
        }
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        S::snapshot(&self.inspect, frame)
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
