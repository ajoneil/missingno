//! The 0840 EconoBanking board (Fred Quimby, homebrew): two banks selected from
//! low memory, on a decode that collapses onto a single address line.
//!
//! The board watches the $0800-$0FFF band and any access there — read or write,
//! data irrelevant — selects. It compares three lines, A12, A11 and A6, but
//! inside the band the first two are already fixed, so the choice reduces to A6
//! alone: there is no inert address in the band at all, and every other line is
//! a don't-care. The only near miss is the band's own twin up in the window,
//! where A12 is high and the board is not listening.

use super::low_bank_select::Decode;

/// A12, A11, A6: the only lines the board compares.
pub(super) const DECODE: Decode = Decode {
    lines: 0x1840,
    bank_0: 0x0800,
    bank_1: 0x0840,
};
