//! The Z80's register file as one inspection group, shared by the live view
//! and the running snapshot.

use missingno_core::inspect::{Register, RegisterGroup, RegisterPurpose, ValueStyle};

use super::Sg1000InspectState;

pub(crate) fn register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
    use RegisterPurpose::{PairHigh, PairLow, ProgramCounter, StackPointer};

    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", state.a as u32, 8)
                .help("accumulator")
                .purpose(PairHigh("af")),
            Register {
                name: "f",
                value: state.f as u32,
                bits: 8,
                style: ValueStyle::Flags(missingno_zilog_z80::flags::NAMES),
                help: Some("flags register"),
                purpose: Some(PairLow("af")),
                active: None,
            },
            hex("b", state.b as u32, 8)
                .help("general register B (high byte of BC)")
                .purpose(PairHigh("bc")),
            hex("c", state.c as u32, 8)
                .help("general register C (low byte of BC)")
                .purpose(PairLow("bc")),
            hex("d", state.d as u32, 8)
                .help("general register D (high byte of DE)")
                .purpose(PairHigh("de")),
            hex("e", state.e as u32, 8)
                .help("general register E (low byte of DE)")
                .purpose(PairLow("de")),
            hex("h", state.h as u32, 8)
                .help("general register H (high byte of HL)")
                .purpose(PairHigh("hl")),
            hex("l", state.l as u32, 8)
                .help("general register L (low byte of HL)")
                .purpose(PairLow("hl")),
            hex("ix", state.ix as u32, 16).help("index register IX"),
            hex("iy", state.iy as u32, 16).help("index register IY"),
            hex("sp", state.sp as u32, 16)
                .help("stack pointer")
                .purpose(StackPointer),
            hex("pc", state.pc as u32, 16)
                .help("program counter")
                .purpose(ProgramCounter),
        ],
    }]
}

#[cfg(test)]
mod tests {
    use missingno_core::inspect::SectionBlock;

    use super::*;
    use crate::debug::fixtures::power_on_state;

    #[test]
    fn the_cpu_section_sets_the_pointers_and_pairs_apart_from_the_file() {
        let state = Sg1000InspectState {
            pc: 0x1234,
            sp: 0xDFF0,
            a: 0x5A,
            f: 0x0F,
            b: 0xC0,
            c: 0xDE,
            ..power_on_state()
        };
        let section = missingno_core::inspect::cpu_section(register_groups(&state));
        assert_eq!(section.summary, "pc 1234 · sp DFF0");

        let pointers: Vec<&str> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Pointers(pointers) => Some(pointers),
                _ => None,
            })
            .flatten()
            .map(|pointer| pointer.register.name)
            .collect();
        assert_eq!(pointers, ["pc", "sp"]);

        let pairs: Vec<u32> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Pairs(pairs) => Some(pairs),
                _ => None,
            })
            .flatten()
            .map(|pair| pair.combined())
            .collect();
        assert_eq!(pairs, [0x5A0F, 0xC0DE, 0x0000, 0x0000]);

        let file: Vec<&str> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Registers(group) => Some(&group.registers),
                _ => None,
            })
            .flatten()
            .map(|register| register.name)
            .collect();
        assert_eq!(file, ["ix", "iy"]);
    }
}
