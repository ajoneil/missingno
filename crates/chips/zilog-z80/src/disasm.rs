//! Z80 disassembly, algorithmic like the execution decoder: opcodes split
//! into the octal fields `x`(7-6) `y`(5-3) `z`(2-0) with `p`(y>>1) and
//! `q`(y&1), and the instruction groups key off those — including the
//! CB/ED and DD/FD (IX/IY) prefixes, the DDCB/FDCB displacement-first
//! form, and the undocumented half-index registers, `sll`, and
//! bit-op-with-copy instructions.
//!
//! Display follows Zilog syntax with the sibling crates' conventions:
//! lowercase mnemonics, `$`-prefixed lowercase hex, relative branches
//! resolved to absolute targets. Invalid encodings (a dropped DD/FD
//! prefix, unassigned ED pages) render as `noni` — no operation, no
//! interrupt — matching what the silicon executes.

use missingno_core::isa::Flow;

use crate::decode::Fields;

pub struct Disassembly {
    pub mnemonic: String,
    /// Instruction length in bytes (1-4), prefixes included.
    pub length: u8,
    pub flow: Flow,
}

/// Disassemble the instruction whose first byte sits at `address`; `bytes`
/// are up to four bytes starting there. Short reads (the address space
/// ending) decode as if padded with zeroes.
pub fn disassemble(address: u16, bytes: &[u8]) -> Disassembly {
    let mut r = Reader { bytes, pos: 0 };
    match r.byte() {
        0xCB => cb(&mut r),
        0xED => ed(&mut r),
        0xDD => prefixed(address, &mut r, Idx::Ix),
        0xFD => prefixed(address, &mut r, Idx::Iy),
        op => unprefixed(address, &mut r, op, Idx::Hl),
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        let byte = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        byte
    }

    fn peek(&self) -> u8 {
        self.bytes.get(self.pos).copied().unwrap_or(0)
    }

    fn word(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
    }

    fn len(&self) -> u8 {
        self.pos as u8
    }
}

/// Which register the HL slot names: the bare instruction set, or its
/// image under a DD (IX) or FD (IY) prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Idx {
    Hl,
    Ix,
    Iy,
}

impl Idx {
    fn hl(self) -> &'static str {
        match self {
            Idx::Hl => "hl",
            Idx::Ix => "ix",
            Idx::Iy => "iy",
        }
    }
}

/// The single-register file in operand order, `(hl)` in slot 6.
const R: [&str; 8] = ["b", "c", "d", "e", "h", "l", "(hl)", "a"];
const CC: [&str; 8] = ["nz", "z", "nc", "c", "po", "pe", "p", "m"];
const ROT: [&str; 8] = ["rlc", "rrc", "rl", "rr", "sla", "sra", "sll", "srl"];

/// Register-pair table with SP (`rp`), or with AF (`rp2`).
fn rp(p: u8, idx: Idx) -> &'static str {
    ["bc", "de", idx.hl(), "sp"][p as usize]
}

fn rp2(p: u8, idx: Idx) -> &'static str {
    ["bc", "de", idx.hl(), "af"][p as usize]
}

/// Slot-`i` register under an index prefix: H and L read as the
/// undocumented half-index registers when no `(hl)` operand is in play.
fn reg(i: u8, idx: Idx) -> &'static str {
    match (idx, i) {
        (Idx::Ix, 4) => "ixh",
        (Idx::Ix, 5) => "ixl",
        (Idx::Iy, 4) => "iyh",
        (Idx::Iy, 5) => "iyl",
        _ => R[i as usize],
    }
}

/// The HL-slot memory operand: `(hl)`, or `(ix+d)`/`(iy+d)` with the
/// displacement read here (it follows the opcode in every non-CB form).
fn mem(r: &mut Reader, idx: Idx) -> String {
    match idx {
        Idx::Hl => "(hl)".into(),
        _ => indexed(idx, r.byte() as i8),
    }
}

fn indexed(idx: Idx, d: i8) -> String {
    if d < 0 {
        format!("({}-${:02x})", idx.hl(), -(d as i16))
    } else {
        format!("({}+${:02x})", idx.hl(), d)
    }
}

fn alu(y: u8, operand: &str) -> String {
    match y {
        0 => format!("add a,{operand}"),
        1 => format!("adc a,{operand}"),
        2 => format!("sub {operand}"),
        3 => format!("sbc a,{operand}"),
        4 => format!("and {operand}"),
        5 => format!("xor {operand}"),
        6 => format!("or {operand}"),
        _ => format!("cp {operand}"),
    }
}

fn seq(r: &Reader, mnemonic: impl Into<String>) -> Disassembly {
    Disassembly {
        mnemonic: mnemonic.into(),
        length: r.len(),
        flow: Flow::Sequential,
    }
}

fn flowed(r: &Reader, mnemonic: String, flow: Flow) -> Disassembly {
    Disassembly {
        mnemonic,
        length: r.len(),
        flow,
    }
}

/// A DD/FD prefix: DDCB/FDCB hands off displacement-first, a following
/// prefix byte drops this one as a standalone `noni`, and anything else
/// decodes as the base instruction with HL read as IX/IY.
fn prefixed(address: u16, r: &mut Reader, idx: Idx) -> Disassembly {
    match r.peek() {
        0xCB => {
            r.byte();
            ddcb(r, idx)
        }
        0xDD | 0xED | 0xFD => seq(r, "noni"),
        _ => {
            let op = r.byte();
            unprefixed(address, r, op, idx)
        }
    }
}

fn unprefixed(address: u16, r: &mut Reader, op: u8, idx: Idx) -> Disassembly {
    let f = Fields::new(op);
    match f.x {
        0 => match f.z {
            0 => match f.y {
                0 => seq(r, "nop"),
                1 => seq(r, "ex af,af'"),
                2..=7 => {
                    let d = r.byte() as i8;
                    let target = address.wrapping_add(r.len() as u16).wrapping_add(d as u16);
                    let (mnemonic, flow) = match f.y {
                        2 => (
                            format!("djnz ${target:04x}"),
                            Flow::Branch {
                                target: Some(target as u32),
                            },
                        ),
                        3 => (
                            format!("jr ${target:04x}"),
                            Flow::Jump {
                                target: Some(target as u32),
                            },
                        ),
                        y => (
                            format!("jr {},${target:04x}", CC[y as usize - 4]),
                            Flow::Branch {
                                target: Some(target as u32),
                            },
                        ),
                    };
                    flowed(r, mnemonic, flow)
                }
                _ => unreachable!(),
            },
            1 => match f.q {
                0 => {
                    let word = r.word();
                    seq(r, format!("ld {},${word:04x}", rp(f.p, idx)))
                }
                _ => seq(r, format!("add {},{}", idx.hl(), rp(f.p, idx))),
            },
            2 => {
                let mnemonic = match (f.q, f.p) {
                    (0, 0) => "ld (bc),a".into(),
                    (0, 1) => "ld (de),a".into(),
                    (0, 2) => format!("ld (${:04x}),{}", r.word(), idx.hl()),
                    (0, _) => format!("ld (${:04x}),a", r.word()),
                    (_, 0) => "ld a,(bc)".into(),
                    (_, 1) => "ld a,(de)".into(),
                    (_, 2) => format!("ld {},(${:04x})", idx.hl(), r.word()),
                    (_, _) => format!("ld a,(${:04x})", r.word()),
                };
                seq(r, mnemonic)
            }
            3 => match f.q {
                0 => seq(r, format!("inc {}", rp(f.p, idx))),
                _ => seq(r, format!("dec {}", rp(f.p, idx))),
            },
            4 | 5 => {
                let verb = if f.z == 4 { "inc" } else { "dec" };
                let operand = if f.y == 6 {
                    mem(r, idx)
                } else {
                    reg(f.y, idx).into()
                };
                seq(r, format!("{verb} {operand}"))
            }
            6 => {
                // `ld (ix+d),n` reads the displacement before the literal.
                let operand = if f.y == 6 {
                    mem(r, idx)
                } else {
                    reg(f.y, idx).into()
                };
                let n = r.byte();
                seq(r, format!("ld {operand},${n:02x}"))
            }
            _ => seq(
                r,
                ["rlca", "rrca", "rla", "rra", "daa", "cpl", "scf", "ccf"][f.y as usize],
            ),
        },
        1 => match (f.y, f.z) {
            (6, 6) => seq(r, "halt"),
            // With a `(hl)` operand in play, the other side keeps its
            // plain name — `ld h,(ix+d)`, never `ld ixh,(ix+d)`.
            (6, z) => {
                let dst = mem(r, idx);
                seq(r, format!("ld {dst},{}", R[z as usize]))
            }
            (y, 6) => {
                let src = mem(r, idx);
                seq(r, format!("ld {},{src}", R[y as usize]))
            }
            (y, z) => seq(r, format!("ld {},{}", reg(y, idx), reg(z, idx))),
        },
        2 => {
            let operand = if f.z == 6 {
                mem(r, idx)
            } else {
                reg(f.z, idx).into()
            };
            seq(r, alu(f.y, &operand))
        }
        _ => match f.z {
            0 => flowed(r, format!("ret {}", CC[f.y as usize]), Flow::Return),
            1 => match (f.q, f.p) {
                (0, p) => seq(r, format!("pop {}", rp2(p, idx))),
                (_, 0) => flowed(r, "ret".into(), Flow::Return),
                (_, 1) => seq(r, "exx"),
                (_, 2) => flowed(r, format!("jp ({})", idx.hl()), Flow::Jump { target: None }),
                (_, _) => seq(r, format!("ld sp,{}", idx.hl())),
            },
            2 => {
                let word = r.word();
                flowed(
                    r,
                    format!("jp {},${word:04x}", CC[f.y as usize]),
                    Flow::Branch {
                        target: Some(word as u32),
                    },
                )
            }
            3 => match f.y {
                0 => {
                    let word = r.word();
                    flowed(
                        r,
                        format!("jp ${word:04x}"),
                        Flow::Jump {
                            target: Some(word as u32),
                        },
                    )
                }
                2 => {
                    let n = r.byte();
                    seq(r, format!("out (${n:02x}),a"))
                }
                3 => {
                    let n = r.byte();
                    seq(r, format!("in a,(${n:02x})"))
                }
                4 => seq(r, format!("ex (sp),{}", idx.hl())),
                5 => seq(r, "ex de,hl"), // never index-substituted
                6 => seq(r, "di"),
                7 => seq(r, "ei"),
                // y=1 is the CB prefix, intercepted before this decoder.
                _ => seq(r, "noni"),
            },
            4 => {
                let word = r.word();
                flowed(
                    r,
                    format!("call {},${word:04x}", CC[f.y as usize]),
                    Flow::Call {
                        target: Some(word as u32),
                    },
                )
            }
            5 => match (f.q, f.p) {
                (0, p) => seq(r, format!("push {}", rp2(p, idx))),
                (_, 0) => {
                    let word = r.word();
                    flowed(
                        r,
                        format!("call ${word:04x}"),
                        Flow::Call {
                            target: Some(word as u32),
                        },
                    )
                }
                // p=1..3 are the DD/ED/FD prefixes, intercepted earlier.
                _ => seq(r, "noni"),
            },
            6 => {
                let n = r.byte();
                seq(r, alu(f.y, &format!("${n:02x}")))
            }
            _ => {
                let target = (f.y as u32) * 8;
                flowed(
                    r,
                    format!("rst ${target:02x}"),
                    Flow::Call {
                        target: Some(target),
                    },
                )
            }
        },
    }
}

fn cb(r: &mut Reader) -> Disassembly {
    let f = Fields::new(r.byte());
    let operand = R[f.z as usize];
    let mnemonic = match f.x {
        0 => format!("{} {operand}", ROT[f.y as usize]),
        1 => format!("bit {},{operand}", f.y),
        2 => format!("res {},{operand}", f.y),
        _ => format!("set {},{operand}", f.y),
    };
    seq(r, mnemonic)
}

/// DDCB/FDCB: the displacement precedes the final opcode, every operation
/// targets `(ix+d)`, and the undocumented non-`(hl)` slots copy the result
/// into a register — rendered as a second operand.
fn ddcb(r: &mut Reader, idx: Idx) -> Disassembly {
    let d = r.byte() as i8;
    let f = Fields::new(r.byte());
    let target = indexed(idx, d);
    let copy = |s: String| {
        if f.z == 6 {
            s
        } else {
            format!("{s},{}", R[f.z as usize])
        }
    };
    let mnemonic = match f.x {
        0 => copy(format!("{} {target}", ROT[f.y as usize])),
        1 => format!("bit {},{target}", f.y),
        2 => copy(format!("res {},{target}", f.y)),
        _ => copy(format!("set {},{target}", f.y)),
    };
    seq(r, mnemonic)
}

fn ed(r: &mut Reader) -> Disassembly {
    let f = Fields::new(r.byte());
    match f.x {
        1 => match f.z {
            0 => match f.y {
                6 => seq(r, "in (c)"),
                y => seq(r, format!("in {},(c)", R[y as usize])),
            },
            1 => match f.y {
                6 => seq(r, "out (c),0"),
                y => seq(r, format!("out (c),{}", R[y as usize])),
            },
            2 => match f.q {
                0 => seq(r, format!("sbc hl,{}", rp(f.p, Idx::Hl))),
                _ => seq(r, format!("adc hl,{}", rp(f.p, Idx::Hl))),
            },
            3 => {
                let word = r.word();
                match f.q {
                    0 => seq(r, format!("ld (${word:04x}),{}", rp(f.p, Idx::Hl))),
                    _ => seq(r, format!("ld {},(${word:04x})", rp(f.p, Idx::Hl))),
                }
            }
            4 => seq(r, "neg"),
            5 => match f.y {
                1 => flowed(r, "reti".into(), Flow::Return),
                _ => flowed(r, "retn".into(), Flow::Return),
            },
            6 => seq(
                r,
                format!("im {}", ["0", "0/1", "1", "2"][(f.y & 3) as usize]),
            ),
            _ => match f.y {
                0 => seq(r, "ld i,a"),
                1 => seq(r, "ld r,a"),
                2 => seq(r, "ld a,i"),
                3 => seq(r, "ld a,r"),
                4 => seq(r, "rrd"),
                5 => seq(r, "rld"),
                _ => seq(r, "noni"),
            },
        },
        2 if f.z <= 3 && f.y >= 4 => {
            let block = [
                ["ldi", "cpi", "ini", "outi"],
                ["ldd", "cpd", "ind", "outd"],
                ["ldir", "cpir", "inir", "otir"],
                ["lddr", "cpdr", "indr", "otdr"],
            ];
            seq(r, block[f.y as usize - 4][f.z as usize])
        }
        _ => seq(r, "noni"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(address: u16, bytes: &[u8]) -> (String, u8) {
        let d = disassemble(address, bytes);
        (d.mnemonic, d.length)
    }

    fn text(bytes: &[u8]) -> String {
        dis(0, bytes).0
    }

    #[test]
    fn unprefixed_instructions() {
        assert_eq!(dis(0, &[0x00]), ("nop".into(), 1));
        assert_eq!(text(&[0x08]), "ex af,af'");
        assert_eq!(dis(0, &[0x01, 0x34, 0x12]), ("ld bc,$1234".into(), 3));
        assert_eq!(text(&[0x22, 0x00, 0xc0]), "ld ($c000),hl");
        assert_eq!(text(&[0x3a, 0x00, 0xc0]), "ld a,($c000)");
        assert_eq!(dis(0, &[0x36, 0x42]), ("ld (hl),$42".into(), 2));
        assert_eq!(text(&[0x6c]), "ld l,h");
        assert_eq!(text(&[0x76]), "halt");
        assert_eq!(text(&[0x86]), "add a,(hl)");
        assert_eq!(text(&[0x97]), "sub a");
        assert_eq!(text(&[0xfe, 0x2a]), "cp $2a");
        assert_eq!(text(&[0x33]), "inc sp");
        assert_eq!(text(&[0x27]), "daa");
        assert_eq!(text(&[0xd3, 0x10]), "out ($10),a");
        assert_eq!(text(&[0xdb, 0x10]), "in a,($10)");
        assert_eq!(text(&[0xf1]), "pop af");
        assert_eq!(text(&[0xe5]), "push hl");
        assert_eq!(text(&[0xf9]), "ld sp,hl");
        assert_eq!(text(&[0xe3]), "ex (sp),hl");
        assert_eq!(text(&[0xeb]), "ex de,hl");
        assert_eq!(text(&[0xd9]), "exx");
    }

    #[test]
    fn relative_targets_resolve_absolute() {
        assert_eq!(dis(0x0100, &[0x18, 0xfe]), ("jr $0100".into(), 2));
        assert_eq!(dis(0x0100, &[0x10, 0x00]), ("djnz $0102".into(), 2));
        assert_eq!(dis(0x0100, &[0x20, 0x05]), ("jr nz,$0107".into(), 2));
        assert_eq!(dis(0x0100, &[0x38, 0xf0]), ("jr c,$00f2".into(), 2));
    }

    #[test]
    fn control_flow_classification() {
        let flow = |bytes: &[u8]| disassemble(0, bytes).flow;
        assert!(matches!(
            flow(&[0xc3, 0x00, 0x40]),
            Flow::Jump {
                target: Some(0x4000)
            }
        ));
        assert!(matches!(flow(&[0xe9]), Flow::Jump { target: None }));
        assert!(matches!(flow(&[0xdd, 0xe9]), Flow::Jump { target: None }));
        assert!(matches!(
            flow(&[0xca, 0x00, 0x40]),
            Flow::Branch {
                target: Some(0x4000)
            }
        ));
        assert!(matches!(flow(&[0x10, 0x10]), Flow::Branch { .. }));
        assert!(matches!(
            flow(&[0xcd, 0x00, 0x40]),
            Flow::Call {
                target: Some(0x4000)
            }
        ));
        assert!(matches!(
            flow(&[0xc4, 0x00, 0x40]),
            Flow::Call {
                target: Some(0x4000)
            }
        ));
        assert!(matches!(flow(&[0xff]), Flow::Call { target: Some(0x38) }));
        assert!(matches!(flow(&[0xc9]), Flow::Return));
        assert!(matches!(flow(&[0xd8]), Flow::Return));
        assert!(matches!(flow(&[0xed, 0x4d]), Flow::Return));
        assert!(matches!(flow(&[0xed, 0x45]), Flow::Return));
        assert!(matches!(flow(&[0x00]), Flow::Sequential));
    }

    #[test]
    fn cb_prefix() {
        assert_eq!(dis(0, &[0xcb, 0x7c]), ("bit 7,h".into(), 2));
        assert_eq!(text(&[0xcb, 0x06]), "rlc (hl)");
        assert_eq!(text(&[0xcb, 0x37]), "sll a");
        assert_eq!(text(&[0xcb, 0x9e]), "res 3,(hl)");
        assert_eq!(text(&[0xcb, 0xff]), "set 7,a");
    }

    #[test]
    fn ed_prefix() {
        assert_eq!(dis(0, &[0xed, 0xb0]), ("ldir".into(), 2));
        assert_eq!(text(&[0xed, 0xa1]), "cpi");
        assert_eq!(text(&[0xed, 0xbb]), "otdr");
        assert_eq!(text(&[0xed, 0x47]), "ld i,a");
        assert_eq!(text(&[0xed, 0x5f]), "ld a,r");
        assert_eq!(
            dis(0, &[0xed, 0x43, 0x34, 0x12]),
            ("ld ($1234),bc".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xed, 0x7b, 0x34, 0x12]),
            ("ld sp,($1234)".into(), 4)
        );
        assert_eq!(text(&[0xed, 0x44]), "neg");
        assert_eq!(text(&[0xed, 0x42]), "sbc hl,bc");
        assert_eq!(text(&[0xed, 0x6a]), "adc hl,hl");
        assert_eq!(text(&[0xed, 0x40]), "in b,(c)");
        assert_eq!(text(&[0xed, 0x70]), "in (c)");
        assert_eq!(text(&[0xed, 0x71]), "out (c),0");
        assert_eq!(text(&[0xed, 0x46]), "im 0");
        assert_eq!(text(&[0xed, 0x56]), "im 1");
        assert_eq!(text(&[0xed, 0x5e]), "im 2");
        assert_eq!(text(&[0xed, 0x67]), "rrd");
        assert_eq!(dis(0, &[0xed, 0x00]), ("noni".into(), 2));
        assert_eq!(dis(0, &[0xed, 0xff]), ("noni".into(), 2));
    }

    #[test]
    fn index_prefixes() {
        assert_eq!(dis(0, &[0xdd, 0x21, 0x34, 0x12]), ("ld ix,$1234".into(), 4));
        assert_eq!(dis(0, &[0xfd, 0x21, 0x34, 0x12]), ("ld iy,$1234".into(), 4));
        assert_eq!(dis(0, &[0xdd, 0x34, 0x05]), ("inc (ix+$05)".into(), 3));
        assert_eq!(
            dis(0, &[0xdd, 0x36, 0x05, 0x42]),
            ("ld (ix+$05),$42".into(), 4)
        );
        assert_eq!(dis(0, &[0xdd, 0x66, 0xfb]), ("ld h,(ix-$05)".into(), 3));
        assert_eq!(dis(0, &[0xdd, 0x75, 0x00]), ("ld (ix+$00),l".into(), 3));
        assert_eq!(dis(0, &[0xdd, 0x86, 0x7f]), ("add a,(ix+$7f)".into(), 3));
        assert_eq!(dis(0, &[0xdd, 0x65]), ("ld ixh,ixl".into(), 2));
        assert_eq!(dis(0, &[0xfd, 0x7c]), ("ld a,iyh".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0x19]), ("add ix,de".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0x29]), ("add ix,ix".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0xe1]), ("pop ix".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0xe3]), ("ex (sp),ix".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0xeb]), ("ex de,hl".into(), 2));
        assert_eq!(dis(0, &[0xdd, 0xf9]), ("ld sp,ix".into(), 2));
        assert_eq!(
            dis(0, &[0xdd, 0x22, 0x00, 0xc0]),
            ("ld ($c000),ix".into(), 4)
        );
        // A prefix followed by another prefix drops as a standalone noni.
        assert_eq!(dis(0, &[0xdd, 0xdd]), ("noni".into(), 1));
        assert_eq!(dis(0, &[0xdd, 0xed]), ("noni".into(), 1));
        assert_eq!(dis(0, &[0xfd, 0xdd]), ("noni".into(), 1));
    }

    #[test]
    fn ddcb_displacement_first() {
        assert_eq!(
            dis(0, &[0xdd, 0xcb, 0x05, 0x46]),
            ("bit 0,(ix+$05)".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xdd, 0xcb, 0x05, 0x7e]),
            ("bit 7,(ix+$05)".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xdd, 0xcb, 0x05, 0x06]),
            ("rlc (ix+$05)".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xdd, 0xcb, 0x05, 0x00]),
            ("rlc (ix+$05),b".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xfd, 0xcb, 0xff, 0xc6]),
            ("set 0,(iy-$01)".into(), 4)
        );
        assert_eq!(
            dis(0, &[0xfd, 0xcb, 0x00, 0x97]),
            ("res 2,(iy+$00),a".into(), 4)
        );
    }

    #[test]
    fn short_reads_pad_with_zeroes() {
        // A lone 0xC3 at the end of memory still decodes as a 3-byte jp.
        assert_eq!(dis(0, &[0xc3]), ("jp $0000".into(), 3));
        assert_eq!(dis(0, &[0xdd]), ("nop".into(), 2));
    }
}
