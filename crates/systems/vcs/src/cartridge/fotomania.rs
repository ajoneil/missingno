//! The 0FA0 board (the Brazilian Fotomania): two banks selected from low
//! memory, through a decode loose enough to alias each hotspot into a family.
//!
//! Like UA, the selects sit below the window and the board just watches the bus:
//! any access, read or write, flips the bank with the data irrelevant. It
//! examines six address lines and treats the rest as don't-cares, so $06A0,
//! $07A0, $0EA0 and $0FA0 all select bank 0 — the last of them naming the board.
//! An address on those pages that misses the pattern selects nothing.

use super::low_bank_select::Decode;

/// A12, A10, A9, A7, A6, A5: the only lines the board examines.
pub(super) const DECODE: Decode = Decode {
    lines: 0x16E0,
    bank_0: 0x06A0,
    bank_1: 0x06C0,
};
