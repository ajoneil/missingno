//! The UA Ltd board: two 4 KB banks, selected from hotspots that live outside
//! the cartridge window and are decoded loosely enough to alias widely.
//!
//! $0220 selects bank 0 and $0240 bank 1 — down in the low address space, where
//! A12 is low and the cart drives nothing. The port has no chip select, so the
//! board simply watches the bus: any access to a hotspot, read or write, flips
//! the bank with the data value irrelevant.
//!
//! The decode examines only A12, A9, A6 and A5 and treats every other line as a
//! don't-care, so each hotspot is a whole family of aliases rather than one
//! address ($0320 and $02A0 both reduce to $0220).
//!
//! Because the hotspots sit at A12=0 they also land on TIA and RIOT mirrors: a
//! write to $0220 pokes HMP0 as well as paging the bank. The console still
//! routes them there — the board only listens.

use super::low_bank_select::Decode;

/// A12, A9, A6, A5: the only address lines the board examines.
pub(super) const DECODE: Decode = Decode {
    lines: 0x1260,
    bank_0: 0x0220,
    bank_1: 0x0240,
};
