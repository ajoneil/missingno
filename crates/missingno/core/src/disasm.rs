//! Address-walking for a disassembly view: which addresses to show around the
//! program counter, and whether each is code or data. Shared across CPU
//! families through the [`InstructionSet`](crate::isa::InstructionSet)
//! vocabulary — decoding each row for display stays with the caller.
//!
//! Forward from the PC is exact: instructions decode to their length, and a
//! byte the code/data log knows was never executed shows as data. Backward is a
//! guess — nothing records where the instruction before the PC began — so two
//! strategies exist: an exact walk anchored on logged instruction starts, and a
//! heuristic sweep for when the log has no coverage.

use crate::cdl::CdlWindow;
use crate::isa::InstructionSet;

/// A byte-addressable memory the disassembler reads without side effects.
pub trait ReadMemory {
    fn read(&self, address: u32) -> u8;
}

/// One line of the forward disassembly window: an instruction to decode for
/// display, or a byte to show verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Instruction(u32),
    Data(u32),
}

/// The rows from `pc` onwards: `count` lines, each an instruction advanced by
/// its decoded length, except bytes the log flags as data (shown one at a time).
/// Without a log every line is an instruction.
pub fn window_after(
    pc: u32,
    count: usize,
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
    cdl: Option<&CdlWindow>,
) -> Vec<Row> {
    let mask = isa.address_mask();
    // A synthetic bank-complete anchor carries high bits above the ISA space;
    // step the window with wrapping and keep the base so the walk stays inside
    // the same store instead of aliasing back onto the bus.
    let base = pc & !mask;
    let mut window = pc & mask;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let address = base | window;
        if cdl.is_some_and(|log| log.is_data(window as u16)) {
            rows.push(Row::Data(address));
            window = window.wrapping_add(1) & mask;
        } else {
            rows.push(Row::Instruction(address));
            let length = decoded_length(isa, memory, address, mask);
            window = window.wrapping_add(length) & mask;
        }
    }
    rows
}

/// Instruction-aligned addresses before `pc`, found by disassembling forward
/// from each candidate start and keeping the longest chain that lands exactly
/// on `pc`. A guess with no log to anchor it; searches back far enough for
/// `count` instructions, capped so a bad alignment can't run away.
pub fn addresses_before(
    pc: u32,
    count: usize,
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
) -> Vec<u32> {
    let mask = isa.address_mask();
    let base = pc & !mask;
    let pc = pc & mask;
    let search_distance = (count * isa.max_len()).min(128) as u32;
    let start = pc.saturating_sub(search_distance);

    let mut best: Vec<u32> = Vec::new();
    for candidate in start..pc {
        let mut window = candidate;
        let mut chain = Vec::new();
        while window < pc {
            chain.push(base | window);
            let length = decoded_length(isa, memory, base | window, mask);
            window = window.wrapping_add(length) & mask;
        }
        if window == pc && chain.len() >= best.len() {
            best = chain;
        }
    }

    if best.len() > count {
        best.split_off(best.len() - count)
    } else {
        best
    }
}

/// Instruction-aligned addresses before `pc`, anchored on log-flagged
/// instruction starts: the previous instruction is the nearest logged start
/// whose length lands exactly on the current address. Exact where the log has
/// seen execution; `None` when it has not.
pub fn logged_addresses_before(
    pc: u32,
    count: usize,
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
    cdl: &CdlWindow,
) -> Option<Vec<u32>> {
    let mask = isa.address_mask();
    let base = pc & !mask;
    let mut chain = Vec::new();
    let mut current = pc & mask;
    for _ in 0..count {
        let previous = (1..=isa.max_len() as u32)
            .map(|back| current.wrapping_sub(back) & mask)
            .find(|&window| {
                cdl.is_instruction_start(window as u16)
                    && window.wrapping_add(decoded_length(isa, memory, base | window, mask)) & mask
                        == current
            });
        match previous {
            Some(window) => {
                chain.push(base | window);
                current = window;
            }
            None => break,
        }
    }
    if chain.is_empty() {
        return None;
    }
    chain.reverse();
    Some(chain)
}

/// The byte length of the instruction at `address`, at least one so a walk
/// always advances.
fn decoded_length(
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
    address: u32,
    mask: u32,
) -> u32 {
    let base = address & !mask;
    let window = address & mask;
    let bytes: Vec<u8> = (0..isa.max_len())
        .map(|offset| memory.read(base | (window.wrapping_add(offset as u32) & mask)))
        .collect();
    (isa.decode(address, &bytes).length as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdl::{CODE, DATA, INSTRUCTION_START};
    use crate::isa::{Flow, Instruction};

    /// A synthetic ISA where an instruction's length is `opcode % 3 + 1`
    /// (1..=3 bytes), so a stream can be deliberately misaligned.
    struct Toy;
    impl InstructionSet for Toy {
        fn max_len(&self) -> usize {
            3
        }
        fn decode(&self, _address: u32, bytes: &[u8]) -> Instruction {
            let opcode = bytes.first().copied().unwrap_or(0);
            Instruction {
                mnemonic: format!("op{opcode:02x}"),
                length: opcode % 3 + 1,
                flow: Flow::Sequential,
            }
        }
    }

    struct Bytes(Vec<u8>);
    impl ReadMemory for Bytes {
        fn read(&self, address: u32) -> u8 {
            self.0.get(address as usize).copied().unwrap_or(0)
        }
    }

    // A stream of single-byte instructions (opcode 0x00 => length 1) so
    // addresses advance one at a time from a known base.
    fn ones(len: usize) -> Bytes {
        Bytes(vec![0x00; len])
    }

    #[test]
    fn forward_window_advances_by_decoded_length() {
        // 0x02 => 3 bytes, 0x01 => 2 bytes, 0x00 => 1 byte.
        let memory = Bytes(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00]);
        let rows = window_after(0, 3, &Toy, &memory, None);
        assert_eq!(
            rows,
            vec![
                Row::Instruction(0),
                Row::Instruction(3),
                Row::Instruction(5)
            ]
        );
    }

    #[test]
    fn forward_window_shows_logged_data_bytes() {
        // Byte 1 is data (never executed); the rest are one-byte instructions.
        let memory = ones(4);
        let cdl = CdlWindow::new(0, vec![CODE, DATA, CODE, CODE]);
        let rows = window_after(0, 4, &Toy, &memory, Some(&cdl));
        assert_eq!(
            rows,
            vec![
                Row::Instruction(0),
                Row::Data(1),
                Row::Instruction(2),
                Row::Instruction(3),
            ]
        );
    }

    #[test]
    fn heuristic_backward_realigns_a_misaligned_stream() {
        // From base 0: a 3-byte, a 2-byte, then one-byte instructions up to pc=8.
        // The only alignment landing exactly on 8 starts at 0.
        let memory = Bytes(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00, 0x00, 0x00, 0x00]);
        let before = addresses_before(8, 4, &Toy, &memory);
        assert_eq!(before, vec![3, 5, 6, 7]);
    }

    #[test]
    fn heuristic_backward_returns_at_most_count() {
        let memory = ones(16);
        let before = addresses_before(10, 3, &Toy, &memory);
        assert_eq!(before, vec![7, 8, 9]);
    }

    #[test]
    fn logged_backward_anchors_on_instruction_starts() {
        // A 3-byte instruction at 0, a 2-byte at 3, a 1-byte at 5; pc=6.
        // Only 0, 3 and 5 are flagged as instruction starts.
        let memory = Bytes(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00]);
        let flags = vec![
            CODE | INSTRUCTION_START,
            CODE,
            CODE,
            CODE | INSTRUCTION_START,
            CODE,
            CODE | INSTRUCTION_START,
        ];
        let cdl = CdlWindow::new(0, flags);
        let before = logged_addresses_before(6, 4, &Toy, &memory, &cdl);
        assert_eq!(before, Some(vec![0, 3, 5]));
    }

    // A store addressed by the low 16-bit window; the synthetic high bits
    // above the ISA space select which store, and the walk must preserve them.
    struct Windowed(Vec<u8>);
    impl ReadMemory for Windowed {
        fn read(&self, address: u32) -> u8 {
            self.0
                .get((address & 0xFFFF) as usize)
                .copied()
                .unwrap_or(0)
        }
    }

    #[test]
    fn forward_window_preserves_synthetic_base() {
        const BASE: u32 = 0x0200_0000;
        let memory = Windowed(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00]);
        let rows = window_after(BASE, 3, &Toy, &memory, None);
        assert_eq!(
            rows,
            vec![
                Row::Instruction(BASE),
                Row::Instruction(BASE | 3),
                Row::Instruction(BASE | 5),
            ]
        );
    }

    #[test]
    fn heuristic_backward_preserves_synthetic_base() {
        const BASE: u32 = 0x0300_0000;
        let memory = Windowed(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00, 0x00, 0x00, 0x00]);
        let before = addresses_before(BASE | 8, 4, &Toy, &memory);
        assert_eq!(before, vec![BASE | 3, BASE | 5, BASE | 6, BASE | 7]);
    }

    #[test]
    fn logged_backward_preserves_synthetic_base() {
        const BASE: u32 = 0x0200_0000;
        let memory = Windowed(vec![0x02, 0xff, 0xff, 0x01, 0xff, 0x00]);
        let flags = vec![
            CODE | INSTRUCTION_START,
            CODE,
            CODE,
            CODE | INSTRUCTION_START,
            CODE,
            CODE | INSTRUCTION_START,
        ];
        let cdl = CdlWindow::new(0, flags);
        let before = logged_addresses_before(BASE | 6, 4, &Toy, &memory, &cdl);
        assert_eq!(before, Some(vec![BASE, BASE | 3, BASE | 5]));
    }

    #[test]
    fn logged_backward_is_none_without_coverage() {
        let memory = ones(8);
        let cdl = CdlWindow::new(0, vec![0; 8]);
        assert_eq!(logged_addresses_before(6, 4, &Toy, &memory, &cdl), None);
    }

    #[test]
    fn data_classification_needs_data_without_code() {
        let cdl = CdlWindow::new(0, vec![CODE, DATA, CODE | DATA, 0]);
        assert!(!cdl.is_data(0));
        assert!(cdl.is_data(1));
        assert!(!cdl.is_data(2));
        assert!(!cdl.is_data(3));
    }
}
