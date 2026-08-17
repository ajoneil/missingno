//! The stand-in BIOS: our own 6502, assembled here.
//!
//! It holds the two documented entry points, one loader, and the template for
//! the six-byte stub the real BIOS leaves in zero page. The loader reaches the
//! board through [`LOAD_REQUEST`] — a read there names the load it wants in the
//! address's low byte, and answers [`LOADED`] or [`NO_LOAD`] — and the board
//! patches the start address and control byte into the template before the
//! answer comes back.

use super::BIOS_SIZE;

const BASE: u16 = 0xF800;
/// A read in this page asks the board for a load.
pub const LOAD_REQUEST: u16 = BASE + 0x100;
/// The latch page, as the loader addresses it.
const LATCH_PAGE: u16 = 0xF000;
pub const LOADED: u8 = 0xFF;
pub const NO_LOAD: u8 = 0x00;

/// The documented entry points: load, and rewind-then-load.
const LOAD_ENTRY: usize = 0x000;
const REWIND_ENTRY: usize = 0x00A;
const LOADER: usize = 0x020;
/// Where a request no unit answers leaves the machine.
const NO_TAPE: usize = 0x028;
/// `CMP $FFF8 / JMP start`, copied to $FA-$FF; the board patches the start.
const STUB: usize = 0x060;
pub const START_LOW: usize = STUB + 4;
pub const START_HIGH: usize = STUB + 5;
pub const CONTROL_BYTE: usize = 0x066;

/// Zero page: where the game names the load it wants, and where the stub
/// and the control byte the real BIOS leaves behind go.
const REQUEST: u8 = 0xFA;
const STUB_TARGET: u16 = 0x00FA;
const CONTROL_COPY: u8 = 0x80;
/// cntlbyte.doc has the BIOS clear $81-$9D: the top of that run, and the
/// byte below it the loop stops on.
const CLEAR_TOP: u8 = 0x9D;
const CLEAR_BELOW: u8 = 0x80;

/// The accumulator the real BIOS hands over is seeded from tape timing.
const SEED: u8 = 0x00;

/// Nothing outside the entry points is callable.
const JAM: u8 = 0x02;

const fn address(offset: usize) -> u16 {
    BASE + offset as u16
}

fn jmp(target: u16) -> [u8; 3] {
    [0x4C, target as u8, (target >> 8) as u8]
}

pub fn assemble() -> [u8; BIOS_SIZE] {
    let mut bios = [JAM; BIOS_SIZE];

    // A rewind seeks a tape position nothing here has, so both entries run
    // the one loader.
    bios[LOAD_ENTRY..LOAD_ENTRY + 3].copy_from_slice(&jmp(address(LOADER)));
    bios[REWIND_ENTRY..REWIND_ENTRY + 3].copy_from_slice(&jmp(address(LOADER)));

    let loader = [
        &[0xA5, REQUEST][..],                 // lda $FA
        &[0xAA],                              // tax
        &lda_absolute_x(LOAD_REQUEST),        // lda $F900,x
        &[0xD0, 0x03],                        // bne +3
        &jmp(address(NO_TAPE)),               // jmp *  (no tape)
        &[0xA2, 0x05],                        // ldx #5
        &lda_absolute_x(address(STUB)),       // lda stub,x
        &[0x95, REQUEST],                     // sta $FA,x
        &[0xCA],                              // dex
        &[0x10, 0xF8],                        // bpl -8
        &[0xA9, 0x00],                        // lda #0
        &[0xA2, CLEAR_TOP],                   // ldx #$9D
        &[0x95, 0x00],                        // sta $00,x
        &[0xCA],                              // dex
        &[0xE0, CLEAR_BELOW],                 // cpx #$80
        &[0xD0, 0xF9],                        // bne -7
        &lda_absolute(address(CONTROL_BYTE)), // lda control
        &[0x85, CONTROL_COPY],                // sta $80
        &[0xAA],                              // tax
        &lda_absolute_x(LATCH_PAGE),          // lda $F000,x
        &[0xA9, SEED],                        // lda #seed
        &[0xA2, 0xFF],                        // ldx #$FF
        &[0xA0, 0x00],                        // ldy #0
        &[0x9A],                              // txs
        &jmp(STUB_TARGET),                    // jmp $00FA
    ]
    .concat();
    bios[LOADER..LOADER + loader.len()].copy_from_slice(&loader);

    bios[STUB..STUB + 4].copy_from_slice(&[0xCD, 0xF8, 0xFF, 0x4C]);

    // The reset vector, at the top of the window, boots the first load.
    bios[BIOS_SIZE - 4..BIOS_SIZE - 2].copy_from_slice(&address(LOAD_ENTRY).to_le_bytes());
    bios
}

fn lda_absolute(target: u16) -> [u8; 3] {
    [0xAD, target as u8, (target >> 8) as u8]
}

fn lda_absolute_x(base: u16) -> [u8; 3] {
    [0xBD, base as u8, (base >> 8) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loader_spins_where_a_request_no_unit_answers_leaves_it() {
        let bios = assemble();
        assert_eq!(bios[NO_TAPE..NO_TAPE + 3], jmp(address(NO_TAPE)));
    }
}
