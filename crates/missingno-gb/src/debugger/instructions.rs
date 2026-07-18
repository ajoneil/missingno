use missingno_core::cdl::CdlWindow;
pub use missingno_core::disasm::Row;
use missingno_core::disasm::{self, ReadMemory};

use crate::isa::Sm83;
use crate::{Console, Model};

/// A byte-addressable memory the disassembler can read without side effects:
/// the live [`Console`] when paused, or a copied window around PC when the
/// core is running on the emulation thread. The 16-bit CPU-address view of the
/// core's [`ReadMemory`].
pub trait ReadInstructionMemory {
    fn read(&self, address: u16) -> u8;
}

impl<M: Model> ReadInstructionMemory for Console<M> {
    fn read(&self, address: u16) -> u8 {
        Console::<M>::read(self, address)
    }
}

/// Presents a 16-bit debugger memory to the core's wider address walker.
struct Widened<'a>(&'a dyn ReadInstructionMemory);

impl ReadMemory for Widened<'_> {
    fn read(&self, address: u32) -> u8 {
        self.0.read(address as u16)
    }
}

pub struct InstructionsIterator<'a, R: ReadInstructionMemory + ?Sized> {
    pub address: Option<u16>,
    pub memory: &'a R,
}

impl<'a, R: ReadInstructionMemory + ?Sized> InstructionsIterator<'a, R> {
    pub fn new(address: u16, memory: &'a R) -> Self {
        InstructionsIterator {
            address: Some(address),
            memory,
        }
    }
}

impl<R: ReadInstructionMemory + ?Sized> Iterator for InstructionsIterator<'_, R> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(address) = self.address {
            self.address = Some(address.wrapping_add(1));
            Some(self.memory.read(address))
        } else {
            None
        }
    }
}

/// Instruction-aligned addresses before `pc`: exact where the code/data log has
/// seen execution, falling back to the heuristic sweep where it hasn't.
pub fn addresses_before(
    pc: u16,
    count: usize,
    memory: &dyn ReadInstructionMemory,
    cdl: Option<&CdlWindow>,
) -> Vec<u16> {
    let memory = Widened(memory);
    let addresses = cdl
        .and_then(|log| disasm::logged_addresses_before(pc as u32, count, &Sm83, &memory, log))
        .unwrap_or_else(|| disasm::addresses_before(pc as u32, count, &Sm83, &memory));
    addresses
        .into_iter()
        .map(|address| address as u16)
        .collect()
}

/// The forward disassembly rows from `pc` onwards — instructions advanced by
/// their length, log-flagged data bytes shown one at a time.
pub fn rows_from(
    pc: u16,
    count: usize,
    memory: &dyn ReadInstructionMemory,
    cdl: Option<&CdlWindow>,
) -> Vec<Row> {
    disasm::window_after(pc as u32, count, &Sm83, &Widened(memory), cdl)
}
