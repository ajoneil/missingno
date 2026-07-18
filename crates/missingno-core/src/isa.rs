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

/// The role a disassembly operand plays, for syntax highlighting. The opcode
/// is implicit — it is always the mnemonic's first word. A renderer maps each
/// class to a colour; the class itself names no colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperandClass {
    /// A CPU register name (`a`, `hl`, `x`).
    Register,
    /// A branch condition (`nz`, `c` after a jump).
    Condition,
    /// A literal value (`$3F`, `#$44`, a decimal count).
    Immediate,
    /// A memory reference (`[hl]`, `($1234)`, a 6502 address).
    Memory,
    /// Anything the ISA does not classify.
    Plain,
}

/// A CPU family's decode-for-display front end. Stateless, so a `&'static`
/// reference can ride a per-vblank snapshot onto the UI thread.
pub trait InstructionSet: Send + Sync {
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

    /// The role one operand plays, for syntax highlighting. The default is a
    /// lexical guess: bracketed or parenthesised operands are memory, a `$` or
    /// leading digit is an immediate, anything else is plain. A family with a
    /// register/condition lexicon overrides this.
    fn classify_operand(&self, operand: &str) -> OperandClass {
        let operand = operand.trim();
        if operand.starts_with('[') || operand.starts_with('(') {
            OperandClass::Memory
        } else if operand.starts_with('$')
            || operand.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            OperandClass::Immediate
        } else {
            OperandClass::Plain
        }
    }
}
