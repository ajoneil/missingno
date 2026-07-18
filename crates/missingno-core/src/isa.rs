//! Debugger-facing instruction vocabulary: enough of an instruction to render
//! a disassembly row and to follow control flow, shared across CPU families.
//!
//! This is decode-for-display — it names an instruction's mnemonic, size, and
//! how it moves the program counter. Execution decoders live in each CPU crate
//! and are separate by design: they drive cycle-accurate stepping, carry state
//! this vocabulary deliberately omits, and are not obliged to agree on shape.

/// A decoded instruction, described only as far as a disassembler needs it.
pub struct Instruction {
    pub mnemonic: String,
    pub length: u8,
    pub flow: Flow,
}

/// How an instruction moves the program counter.
pub enum Flow {
    /// Falls through to the following instruction.
    Sequential,
    /// Conditional control transfer.
    Branch { target: Option<u32> },
    /// Unconditional transfer; `None` when the destination is indirect or computed.
    Jump { target: Option<u32> },
    /// Subroutine call.
    Call { target: Option<u32> },
    /// Return from a subroutine or interrupt.
    Return,
}

/// A CPU family's decode-for-display front end.
pub trait InstructionSet {
    /// Trace-format tag identifying this ISA.
    fn id(&self) -> &'static str;

    /// The longest instruction this ISA decodes, in bytes.
    fn max_len(&self) -> usize;

    /// The address bus wrapped to a bit width — a disassembler walking off
    /// either end of memory rolls over here. Defaults to the 16-bit space
    /// every current core uses.
    fn address_mask(&self) -> u32 {
        0xFFFF
    }

    /// Decode the instruction at `address`. Callers supply up to `max_len`
    /// bytes starting at `address`, fewer only when the address space ends
    /// first. This is decode-for-display — execution decoders are separate by
    /// design.
    fn decode(&self, address: u32, bytes: &[u8]) -> Instruction;
}
