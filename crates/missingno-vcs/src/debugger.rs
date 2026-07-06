//! Debugging backend: instruction stepping, PC breakpoints, and bus-access
//! watchpoints over a console, with side-effect-free inspection through
//! [`Vcs::peek`].

use std::collections::BTreeSet;

use crate::console::{Frame, Vcs};

/// A JSR opcode, for step-over.
const JSR: u8 = 0x20;

/// Guard for step_frame: a bit over four NTSC frames' worth of the
/// shortest instructions, so a syncless kernel cannot stall the caller.
const FRAME_INSTRUCTION_BUDGET: u32 = 200_000;

pub struct Debugger {
    vcs: Vcs,
    breakpoints: BTreeSet<u16>,
}

/// Why a stepping call returned.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum Stop {
    FrameComplete,
    Breakpoint,
    BudgetExhausted,
}

impl Debugger {
    pub fn new(vcs: Vcs) -> Self {
        Debugger {
            vcs,
            breakpoints: BTreeSet::new(),
        }
    }

    pub fn console(&self) -> &Vcs {
        &self.vcs
    }

    pub fn console_mut(&mut self) -> &mut Vcs {
        &mut self.vcs
    }

    pub fn into_console(self) -> Vcs {
        self.vcs
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    pub fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    /// The 6507 drives 13 address lines: breakpoints compare on them.
    fn at_breakpoint(&self) -> bool {
        self.breakpoints
            .iter()
            .any(|&bp| bp & 0x1FFF == self.vcs.cpu.pc & 0x1FFF)
    }

    /// Execute one instruction; a frame completing mid-instruction
    /// surfaces here.
    pub fn step(&mut self) -> Option<Frame> {
        self.vcs.step_instruction();
        self.vcs.take_frame()
    }

    /// Like step, but a JSR runs to the instruction after the call
    /// (bounded, and stopping at breakpoints inside the subroutine).
    pub fn step_over(&mut self) -> (Option<Frame>, Stop) {
        if self.vcs.peek(self.vcs.cpu.pc) != JSR {
            let frame = self.step();
            return (frame, Stop::FrameComplete);
        }
        let return_address = self.vcs.cpu.pc.wrapping_add(3);
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            frame = self.vcs.take_frame().or(frame);
            if self.vcs.cpu.pc & 0x1FFF == return_address & 0x1FFF {
                return (frame, Stop::FrameComplete);
            }
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
        }
        (frame, Stop::BudgetExhausted)
    }

    /// Run until the next frame completes or a breakpoint is hit.
    pub fn step_frame(&mut self) -> (Option<Frame>, Stop) {
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            if let Some(frame) = self.vcs.take_frame() {
                return (Some(frame), Stop::FrameComplete);
            }
            if self.at_breakpoint() {
                return (None, Stop::Breakpoint);
            }
        }
        (None, Stop::BudgetExhausted)
    }

    /// Run until a breakpoint (or budget); frames surface as they complete.
    pub fn run(&mut self) -> (Option<Frame>, Stop) {
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            frame = self.vcs.take_frame().or(frame);
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
        }
        (frame, Stop::BudgetExhausted)
    }
}
