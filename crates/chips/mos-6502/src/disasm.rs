//! 6502 disassembly from the decode table.

use super::decode::{DECODE, Flag, Mode, Op};

pub struct Disassembly {
    pub mnemonic: String,
    /// Instruction length in bytes (1-3).
    pub length: u8,
}

pub fn instruction_length(opcode: u8) -> u8 {
    match DECODE[opcode as usize].mode {
        Mode::Implied | Mode::Accumulator => 1,
        Mode::Immediate
        | Mode::ZeroPage
        | Mode::ZeroPageX
        | Mode::ZeroPageY
        | Mode::IndirectX
        | Mode::IndirectY
        | Mode::Relative => 2,
        Mode::Absolute | Mode::AbsoluteX | Mode::AbsoluteY | Mode::Indirect => 3,
    }
}

/// Disassemble the instruction whose opcode sits at `address`; `bytes` are
/// the opcode and up to two operand bytes.
pub fn disassemble(address: u16, bytes: [u8; 3]) -> Disassembly {
    let instr = DECODE[bytes[0] as usize];
    let length = instruction_length(bytes[0]);
    let operand_byte = bytes[1];
    let operand_word = u16::from_le_bytes([bytes[1], bytes[2]]);
    let name = name(instr.op);
    let mnemonic = match instr.mode {
        Mode::Implied => name.to_string(),
        Mode::Accumulator => format!("{name} a"),
        Mode::Immediate => format!("{name} #${operand_byte:02x}"),
        Mode::ZeroPage => format!("{name} ${operand_byte:02x}"),
        Mode::ZeroPageX => format!("{name} ${operand_byte:02x},x"),
        Mode::ZeroPageY => format!("{name} ${operand_byte:02x},y"),
        Mode::Absolute => format!("{name} ${operand_word:04x}"),
        Mode::AbsoluteX => format!("{name} ${operand_word:04x},x"),
        Mode::AbsoluteY => format!("{name} ${operand_word:04x},y"),
        Mode::IndirectX => format!("{name} (${operand_byte:02x},x)"),
        Mode::IndirectY => format!("{name} (${operand_byte:02x}),y"),
        Mode::Indirect => format!("{name} (${operand_word:04x})"),
        Mode::Relative => {
            let target = address
                .wrapping_add(2)
                .wrapping_add(operand_byte as i8 as u16);
            format!("{name} ${target:04x}")
        }
    };
    Disassembly { mnemonic, length }
}

fn name(op: Op) -> &'static str {
    use Op::*;
    match op {
        Lda => "lda",
        Ldx => "ldx",
        Ldy => "ldy",
        Sta => "sta",
        Stx => "stx",
        Sty => "sty",
        Tax => "tax",
        Tay => "tay",
        Txa => "txa",
        Tya => "tya",
        Tsx => "tsx",
        Txs => "txs",
        Pha => "pha",
        Php => "php",
        Pla => "pla",
        Plp => "plp",
        Adc => "adc",
        Sbc => "sbc",
        And => "and",
        Ora => "ora",
        Eor => "eor",
        Cmp => "cmp",
        Cpx => "cpx",
        Cpy => "cpy",
        Bit => "bit",
        Asl => "asl",
        Lsr => "lsr",
        Rol => "rol",
        Ror => "ror",
        Inc => "inc",
        Dec => "dec",
        Inx => "inx",
        Iny => "iny",
        Dex => "dex",
        Dey => "dey",
        Clc => "clc",
        Sec => "sec",
        Cli => "cli",
        Sei => "sei",
        Clv => "clv",
        Cld => "cld",
        Sed => "sed",
        Jmp => "jmp",
        Jsr => "jsr",
        Rts => "rts",
        Rti => "rti",
        Brk => "brk",
        Nop => "nop",
        Branch(flag, expected) => match (flag, expected) {
            (Flag::Negative, false) => "bpl",
            (Flag::Negative, true) => "bmi",
            (Flag::Overflow, false) => "bvc",
            (Flag::Overflow, true) => "bvs",
            (Flag::Carry, false) => "bcc",
            (Flag::Carry, true) => "bcs",
            (Flag::Zero, false) => "bne",
            (Flag::Zero, true) => "beq",
        },
        Lax => "lax",
        Sax => "sax",
        Dcp => "dcp",
        Isc => "isc",
        Slo => "slo",
        Rla => "rla",
        Sre => "sre",
        Rra => "rra",
        Anc => "anc",
        Alr => "alr",
        Arr => "arr",
        Ane => "ane",
        Lxa => "lxa",
        Sbx => "sbx",
        Sha => "sha",
        Shx => "shx",
        Shy => "shy",
        Tas => "tas",
        Las => "las",
        Jam => "jam",
    }
}
