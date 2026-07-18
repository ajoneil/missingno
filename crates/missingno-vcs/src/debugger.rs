//! Debugging backend: instruction stepping, PC breakpoints, and bus-access
//! watchpoints over a console, with side-effect-free inspection through
//! [`Vcs::peek`].

use std::collections::BTreeSet;

use missingno_6502::Mos6502;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;

use crate::console::{Frame, Vcs};

/// A JSR opcode, for step-over.
const JSR: u8 = 0x20;

/// Named bits of the 6502 status register `p`; the B flag is not architectural.
const MOS6502_FLAGS: &[inspect::FlagName] = &[
    inspect::FlagName { name: "n", bit: 7 },
    inspect::FlagName { name: "v", bit: 6 },
    inspect::FlagName { name: "d", bit: 3 },
    inspect::FlagName { name: "i", bit: 2 },
    inspect::FlagName { name: "z", bit: 1 },
    inspect::FlagName { name: "c", bit: 0 },
];

/// Bounds a syncless kernel: ~20 NTSC frames of minimum-length instructions.
const FRAME_INSTRUCTION_BUDGET: u32 = 200_000;

/// The 6507 register file as one inspection group. Shared by the live debugger
/// and the running snapshot so both produce identical groups.
pub fn cpu_register_groups(
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    s: u8,
    p: u8,
) -> Vec<inspect::RegisterGroup> {
    let hex = |name, value: u32, bits| inspect::Register {
        name,
        value,
        bits,
        style: inspect::ValueStyle::Hex,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("pc", pc as u32, 16),
            hex("a", a as u32, 8),
            hex("x", x as u32, 8),
            hex("y", y as u32, 8),
            hex("s", s as u32, 8),
            inspect::Register {
                name: "p",
                value: p as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(MOS6502_FLAGS),
            },
        ],
    }]
}

pub struct Debugger {
    vcs: Vcs,
    breakpoints: BTreeSet<u16>,
}

/// Why a stepping call returned.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum Stop {
    Completed,
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
            return (frame, Stop::Completed);
        }
        let return_address = self.vcs.cpu.pc.wrapping_add(3);
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            // Keep the newest frame completed while stepping.
            frame = self.vcs.take_frame().or(frame);
            if self.vcs.cpu.pc & 0x1FFF == return_address & 0x1FFF {
                return (frame, Stop::Completed);
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
                return (Some(frame), Stop::Completed);
            }
            if self.at_breakpoint() {
                return (None, Stop::Breakpoint);
            }
        }
        (None, Stop::BudgetExhausted)
    }

    /// The 6502 register file as one inspection group.
    pub fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        let cpu = &self.vcs.cpu;
        cpu_register_groups(cpu.pc, cpu.a, cpu.x, cpu.y, cpu.s, cpu.p)
    }

    /// The 6507's 13-line address map, named for what the board decodes.
    pub fn memory_regions(&self) -> &'static [inspect::MemoryRegion] {
        const fn region(name: &'static str, start: u32, len: u32) -> inspect::MemoryRegion {
            inspect::MemoryRegion { name, start, len }
        }
        static REGIONS: &[inspect::MemoryRegion] = &[
            region("tia", 0x0000, 0x40),
            region("riot-ram", 0x0080, 0x80),
            region("riot-io", 0x0280, 0x20),
            region("cartridge", 0x1000, 0x1000),
        ];
        REGIONS
    }

    /// Side-effect-free read of the 13-bit address space.
    pub fn peek(&self, address: u32) -> u8 {
        self.vcs.peek(address as u16)
    }

    pub fn pc(&self) -> u32 {
        self.vcs.cpu.pc as u32
    }

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Mos6502
    }

    /// Run until a breakpoint (or budget); frames surface as they complete.
    pub fn run(&mut self) -> (Option<Frame>, Stop) {
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            // Keep the newest frame completed while stepping.
            frame = self.vcs.take_frame().or(frame);
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
        }
        (frame, Stop::BudgetExhausted)
    }
}
