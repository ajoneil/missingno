use missingno_core::inspect;

use crate::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use crate::interrupts;

/// The CPU register state the sidebar draws — live [`Cpu`] or a snapshot copy.
pub trait CpuSource {
    fn get_register8(&self, register: Register8) -> u8;
    fn get_register16(&self, register: Register16) -> u16;
    fn flags(&self) -> Flags;
    fn ir_address(&self) -> u16;
    fn stack_pointer(&self) -> u16;
    fn halted(&self) -> bool;
    fn interrupts_enabled(&self) -> bool;
}

impl CpuSource for Cpu {
    fn get_register8(&self, register: Register8) -> u8 {
        Cpu::get_register8(self, register)
    }
    fn get_register16(&self, register: Register16) -> u16 {
        Cpu::get_register16(self, register)
    }
    fn flags(&self) -> Flags {
        self.flags
    }
    fn ir_address(&self) -> u16 {
        self.ir_address
    }
    fn stack_pointer(&self) -> u16 {
        self.stack_pointer
    }
    fn halted(&self) -> bool {
        self.halt.state == HaltState::Halted
    }
    fn interrupts_enabled(&self) -> bool {
        Cpu::interrupts_enabled(self)
    }
}

#[derive(Clone)]
pub struct CpuView {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    flags: Flags,
    pub(super) stack_pointer: u16,
    pub(super) ir_address: u16,
    halted: bool,
    ime: bool,
}

impl CpuView {
    pub(super) fn capture(cpu: &impl CpuSource) -> Self {
        Self {
            a: cpu.get_register8(Register8::A),
            b: cpu.get_register8(Register8::B),
            c: cpu.get_register8(Register8::C),
            d: cpu.get_register8(Register8::D),
            e: cpu.get_register8(Register8::E),
            h: cpu.get_register8(Register8::H),
            l: cpu.get_register8(Register8::L),
            flags: cpu.flags(),
            stack_pointer: cpu.stack_pointer(),
            ir_address: cpu.ir_address(),
            halted: cpu.halted(),
            ime: cpu.interrupts_enabled(),
        }
    }
}

impl CpuSource for CpuView {
    fn get_register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }
    fn get_register16(&self, register: Register16) -> u16 {
        match register {
            Register16::Bc => u16::from_be_bytes([self.b, self.c]),
            Register16::De => u16::from_be_bytes([self.d, self.e]),
            Register16::Hl => u16::from_be_bytes([self.h, self.l]),
            Register16::StackPointer => self.stack_pointer,
            Register16::Af => u16::from_be_bytes([self.a, self.flags.bits()]),
        }
    }
    fn flags(&self) -> Flags {
        self.flags
    }
    fn ir_address(&self) -> u16 {
        self.ir_address
    }
    fn stack_pointer(&self) -> u16 {
        self.stack_pointer
    }
    fn halted(&self) -> bool {
        self.halted
    }
    fn interrupts_enabled(&self) -> bool {
        self.ime
    }
}

/// Named bits of the SM83 flags register `f`.
const SM83_FLAGS: &[inspect::FlagName] = &[
    inspect::FlagName {
        name: "z",
        bit: 7,
        help: Some("zero flag — set when a result is zero"),
    },
    inspect::FlagName {
        name: "n",
        bit: 6,
        help: Some("subtract flag — set by a subtraction (used by DAA)"),
    },
    inspect::FlagName {
        name: "h",
        bit: 5,
        help: Some("half-carry flag — carry out of bit 3 (used by DAA)"),
    },
    inspect::FlagName {
        name: "c",
        bit: 4,
        help: Some("carry flag — set on carry or borrow"),
    },
];

/// The SM83 register file as one inspection group. Shared by the live debugger
/// (over the console's CPU) and the running snapshot (over its captured view),
/// so both produce identical groups. `pc` follows the debugger's convention of
/// the current instruction's fetch address.
pub fn cpu_register_groups(cpu: &impl CpuSource) -> Vec<inspect::RegisterGroup> {
    use inspect::RegisterPurpose::{PairHigh, PairLow, ProgramCounter, StackPointer};

    let hex8 = |name, register| inspect::Register {
        name,
        value: cpu.get_register8(register) as u32,
        bits: 8,
        style: inspect::ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    let hex16 = |name, value: u16| inspect::Register {
        name,
        value: value as u32,
        bits: 16,
        style: inspect::ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex8("a", Register8::A)
                .help("accumulator")
                .purpose(PairHigh("af")),
            inspect::Register {
                name: "f",
                value: cpu.flags().bits() as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(SM83_FLAGS),
                help: Some("flags register"),
                purpose: Some(PairLow("af")),
                active: None,
            },
            hex8("b", Register8::B)
                .help("general register B (high byte of BC)")
                .purpose(PairHigh("bc")),
            hex8("c", Register8::C)
                .help("general register C (low byte of BC)")
                .purpose(PairLow("bc")),
            hex8("d", Register8::D)
                .help("general register D (high byte of DE)")
                .purpose(PairHigh("de")),
            hex8("e", Register8::E)
                .help("general register E (low byte of DE)")
                .purpose(PairLow("de")),
            hex8("h", Register8::H)
                .help("general register H (high byte of HL)")
                .purpose(PairHigh("hl")),
            hex8("l", Register8::L)
                .help("general register L (low byte of HL)")
                .purpose(PairLow("hl")),
            hex16("sp", cpu.stack_pointer())
                .help("stack pointer")
                .purpose(StackPointer),
            hex16("pc", cpu.ir_address())
                .help("program counter")
                .purpose(ProgramCounter)
                .active(!cpu.halted()),
        ],
    }]
}

/// The five interrupt sources, in the order the interrupt table's columns show
/// them.
const INTERRUPT_SOURCES: [interrupts::Interrupt; 5] = [
    interrupts::Interrupt::VideoBetweenFrames,
    interrupts::Interrupt::VideoStatus,
    interrupts::Interrupt::Timer,
    interrupts::Interrupt::Serial,
    interrupts::Interrupt::Joypad,
];

/// The CPU section's collapsed summary.
pub fn cpu_summary(cpu: &impl CpuSource) -> String {
    inspect::register_file_summary(&cpu_register_groups(cpu))
}

/// The shared CPU block list: the register file's derived layout followed by
/// the interrupt table.
pub fn cpu_blocks(
    cpu: &impl CpuSource,
    ints: &interrupts::Registers,
) -> Vec<inspect::SectionBlock> {
    let mut blocks = inspect::register_file_blocks(cpu_register_groups(cpu));
    blocks.push(inspect::SectionBlock::Rule);
    blocks.push(inspect::SectionBlock::Table(interrupt_table(
        ints,
        cpu.interrupts_enabled(),
    )));
    blocks
}

fn interrupt_table(ints: &interrupts::Registers, ime: bool) -> inspect::BitTable {
    use inspect::Concept;
    inspect::BitTable {
        columns: vec![
            inspect::BitColumn::concept("VBlank", Concept::VBlank),
            inspect::BitColumn::concept("Stat", Concept::VideoStatus),
            inspect::BitColumn::concept("Timer", Concept::Timer),
            inspect::BitColumn::concept("Serial", Concept::Serial),
            inspect::BitColumn::concept("Joypad", Concept::Input),
        ],
        corner: Some(inspect::Flag {
            name: "IME",
            active: ime,
        }),
        rows: vec![
            inspect::BitRow {
                name: "IE",
                bits: INTERRUPT_SOURCES.iter().map(|&i| ints.enabled(i)).collect(),
                tone: inspect::Tone::Neutral,
            },
            inspect::BitRow {
                name: "IF",
                bits: INTERRUPT_SOURCES
                    .iter()
                    .map(|&i| ints.requested(i))
                    .collect(),
                tone: inspect::Tone::Pending,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_table_tracks_enabled_and_requested() {
        use crate::interrupts::{Interrupt, InterruptFlags, Registers};

        let mut ints = Registers::new();
        ints.enabled = InterruptFlags::TIMER | InterruptFlags::JOYPAD;
        ints.request(Interrupt::VideoBetweenFrames);

        let table = interrupt_table(&ints, true);
        let names: Vec<_> = table.columns.iter().map(|column| column.name).collect();
        assert_eq!(names, ["VBlank", "Stat", "Timer", "Serial", "Joypad"]);
        let concepts: Vec<_> = table.columns.iter().map(|column| column.concept).collect();
        assert_eq!(
            concepts,
            [
                Some(inspect::Concept::VBlank),
                Some(inspect::Concept::VideoStatus),
                Some(inspect::Concept::Timer),
                Some(inspect::Concept::Serial),
                Some(inspect::Concept::Input),
            ]
        );
        assert_eq!(
            table.corner.map(|flag| (flag.name, flag.active)),
            Some(("IME", true))
        );
        assert_eq!(table.rows[0].name, "IE");
        assert_eq!(table.rows[0].bits, vec![false, false, true, false, true]);
        assert_eq!(table.rows[0].tone, inspect::Tone::Neutral);
        assert_eq!(table.rows[1].name, "IF");
        assert_eq!(table.rows[1].bits, vec![true, false, false, false, false]);
        assert_eq!(table.rows[1].tone, inspect::Tone::Pending);
    }
}
