//! The 12-key keyboard controller: four scanned rows, three answering columns.

use missingno_core::system::{ControlInput, ControlRole};

use super::Jack;
use crate::riot::Riot;
use crate::tia::Tia;

/// The 12-key keyboard controller. It owns no line passively: the console
/// scans it by pulling the jack's row lines low through port A, and the three
/// column lines answer — two on the jack's pot pins, held high by the
/// controller's own pull-ups, and one on its trigger line. The columns answer
/// at once; the RC settle games are told to wait out is not modelled.
#[derive(Default)]
pub(crate) struct Keypad {
    /// Keys held, bit n = key n: row-major from the top left, so 0-8 are the
    /// digits 1-9 and 9, 10, 11 are `*`, 0, `#`.
    held: u16,
}

const KEYPAD_KEYS: usize = 12;
const KEYPAD_ROWS: usize = 4;
const KEYPAD_COLUMNS: usize = 3;

impl Keypad {
    /// A column goes low only where a held key bridges it to a row the console
    /// is pulling low; the pull-ups hold it high otherwise, including while the
    /// console leaves port A an input and every row floats up.
    fn column_low(&self, column: usize, rows: u8) -> bool {
        (0..KEYPAD_ROWS).any(|row| {
            self.held & 1 << (row * KEYPAD_COLUMNS + column) != 0 && rows & 1 << row == 0
        })
    }

    /// Rows run top to bottom up the jack's nibble (Stella Programmer's Guide),
    /// and the columns leave on the two pot pins and the trigger, left to right
    /// (console and controller schematics).
    pub(super) fn drive_columns(&self, jack: Jack, riot: &Riot, tia: &mut Tia) {
        let rows = jack.nibble_of(riot.port_a_level());
        let pots = jack.pots();
        tia.drive_pot(pots[0], self.column_low(0, rows));
        tia.drive_pot(pots[1], self.column_low(1, rows));
        tia.set_trigger(jack.index(), self.column_low(2, rows));
    }

    pub(super) fn apply(
        &mut self,
        jack: Jack,
        role: ControlRole,
        input: ControlInput,
        riot: &mut Riot,
        tia: &mut Tia,
    ) {
        let (ControlRole::Key(key), ControlInput::Digital(pressed)) = (role, input) else {
            return;
        };
        if usize::from(key) >= KEYPAD_KEYS {
            return;
        }
        if pressed {
            self.held |= 1 << key;
        } else {
            self.held &= !(1 << key);
        }
        self.drive_columns(jack, riot, tia);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Vcs;
    use crate::controllers::ControllerKind;
    use crate::controllers::tests::{
        INPT0, INPT1, INPT2, INPT3, INPT4, INPT5, SWACNT, SWCHA, press, release,
    };
    use crate::tia::registers::VBLANK;
    use crate::{DumpFit, TvStandard};

    /// A cart that points port A at the keypad rows and holds one scan pattern
    /// on them, then spins — the scan a keypad game runs before reading the TIA.
    fn scanning_vcs(ddr: u8, rows: u8) -> Vcs {
        // LDA #ddr / STA SWACNT / LDA #rows / STA SWCHA / JMP *
        #[rustfmt::skip]
        let program = [
            0xA9, ddr,
            0x8D, (SWACNT & 0xFF) as u8, (SWACNT >> 8) as u8,
            0xA9, rows,
            0x8D, (SWCHA & 0xFF) as u8, (SWCHA >> 8) as u8,
            0x4C, 0x0A, 0xF0,
        ];
        let mut rom = vec![0xEA; 0x1000];
        rom[..program.len()].copy_from_slice(&program);
        rom[0xFFC] = 0x00;
        rom[0xFFD] = 0xF0;
        Vcs::new(&rom, TvStandard::Ntsc, None, DumpFit::Exact).unwrap()
    }

    /// Run the cart's scan setup to its spin, confirming port A holds it.
    fn run_scan(vcs: &mut Vcs, port_a: u8) {
        for _ in 0..8 {
            vcs.step_instruction();
        }
        assert_eq!(vcs.peek(SWCHA), port_a);
    }

    fn key(n: u8) -> ControlRole {
        ControlRole::Key(n)
    }

    /// The three registers a jack's keypad columns answer on, left to right.
    fn columns(jack: Jack) -> [u16; 3] {
        match jack {
            Jack::Left => [INPT0, INPT1, INPT4],
            Jack::Right => [INPT2, INPT3, INPT5],
        }
    }

    fn reads_low(vcs: &Vcs, register: u16) -> bool {
        vcs.peek(register) & 0x80 == 0
    }

    #[test]
    fn a_keypad_key_answers_on_its_own_column_and_only_on_its_own_row() {
        for jack in [Jack::Left, Jack::Right] {
            let idle = match jack {
                Jack::Left => Jack::Right,
                Jack::Right => Jack::Left,
            };
            for pressed in 0..12u8 {
                let (row, column) = (u32::from(pressed) / 3, usize::from(pressed) % 3);
                for selected in 0..4 {
                    let rows = !jack.port_a(1 << selected);
                    let mut vcs = scanning_vcs(0xFF, rows);
                    vcs.plug(jack, ControllerKind::Keypad);
                    vcs.plug(idle, ControllerKind::Keypad);
                    press(&mut vcs, jack, key(pressed));
                    run_scan(&mut vcs, rows);
                    for (index, register) in columns(jack).into_iter().enumerate() {
                        assert_eq!(
                            reads_low(&vcs, register),
                            index == column && selected == row,
                            "{jack:?} key {pressed}, row {selected} selected, column {index}"
                        );
                    }
                    for register in columns(idle) {
                        assert!(!reads_low(&vcs, register), "{idle:?} answered for {jack:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_key_pressed_mid_scan_answers_without_another_port_write() {
        let rows = !Jack::Left.port_a(0x2);
        let mut vcs = scanning_vcs(0xFF, rows);
        vcs.plug(Jack::Left, ControllerKind::Keypad);
        run_scan(&mut vcs, rows);
        assert!(!reads_low(&vcs, INPT1));
        // Key 5: second row, middle column — the row the scan is holding low.
        press(&mut vcs, Jack::Left, key(4));
        assert!(reads_low(&vcs, INPT1));
        release(&mut vcs, Jack::Left, key(4));
        assert!(!reads_low(&vcs, INPT1));
    }

    #[test]
    fn port_a_left_as_an_input_floats_every_keypad_row_high() {
        let mut vcs = scanning_vcs(0x00, 0x00);
        vcs.plug(Jack::Left, ControllerKind::Keypad);
        for pressed in 0..12u8 {
            press(&mut vcs, Jack::Left, key(pressed));
        }
        run_scan(&mut vcs, 0xFF);
        for register in columns(Jack::Left) {
            assert!(!reads_low(&vcs, register));
        }
    }

    #[test]
    fn the_pot_dump_grounds_a_keypad_column_a_key_is_holding_up() {
        let rows = !Jack::Left.port_a(0x1);
        let mut vcs = scanning_vcs(0xFF, rows);
        vcs.plug(Jack::Left, ControllerKind::Keypad);
        press(&mut vcs, Jack::Left, key(0));
        press(&mut vcs, Jack::Left, key(2));
        run_scan(&mut vcs, rows);
        assert!(reads_low(&vcs, INPT0) && !reads_low(&vcs, INPT1) && reads_low(&vcs, INPT4));

        vcs.tia.write(VBLANK, 0x80);
        assert!(reads_low(&vcs, INPT0) && reads_low(&vcs, INPT1));
        assert!(reads_low(&vcs, INPT4), "the trigger column takes no dump");
    }

    #[test]
    fn unplugging_a_keypad_reopens_its_pots_and_releases_its_trigger() {
        let rows = !Jack::Left.port_a(0x1);
        let mut vcs = scanning_vcs(0xFF, rows);
        vcs.plug(Jack::Left, ControllerKind::Keypad);
        press(&mut vcs, Jack::Left, key(0));
        press(&mut vcs, Jack::Left, key(2));
        run_scan(&mut vcs, rows);
        assert!(reads_low(&vcs, INPT0) && !reads_low(&vcs, INPT1) && reads_low(&vcs, INPT4));

        vcs.plug(Jack::Left, ControllerKind::Unplugged);
        assert!(!reads_low(&vcs, INPT4));
        // An open pot has no charge path at all, so the column that was held up
        // no longer reads high.
        assert!(reads_low(&vcs, INPT0) && reads_low(&vcs, INPT1));
    }
}
