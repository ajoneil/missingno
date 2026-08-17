//! The Z80's register file as one inspection group, shared by the live view
//! and the running snapshot. The part states its own layout.

use missingno_core::inspect::RegisterGroup;

use super::Sg1000InspectState;

pub(crate) fn register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
    missingno_zilog_z80::inspect::register_groups(&state.registers)
}

#[cfg(test)]
mod tests {
    use missingno_core::inspect::SectionBlock;
    use missingno_zilog_z80::inspect::RegisterFile;

    use super::*;
    use crate::debug::fixtures::power_on_state;

    #[test]
    fn the_cpu_section_sets_the_pointers_and_pairs_apart_from_the_file() {
        let powered = power_on_state();
        let state = Sg1000InspectState {
            registers: RegisterFile {
                pc: 0x1234,
                sp: 0xDFF0,
                a: 0x5A,
                f: 0x0F,
                b: 0xC0,
                c: 0xDE,
                ..powered.registers
            },
            ..powered
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
