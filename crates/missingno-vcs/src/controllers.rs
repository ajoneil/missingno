//! What is plugged into the console's two controller jacks.
//!
//! A jack carries four direction lines into the RIOT's port A, a trigger line
//! into the TIA, and two paddle pots. Which of them a peripheral drives — and
//! what a player's input does to them — is the peripheral's business, so each
//! kind holds its own user-facing state and pushes line levels down as that
//! state changes. Nothing behind an empty jack drives anything: the direction
//! lines sit at their pull-ups and the pots have no charge path at all.

use missingno_core::system::{ControlInput, ControlRole};

use crate::riot::Riot;
use crate::tia::Tia;

/// Which of the two jacks a controller sits in. The wiring differs between
/// them, so the jack decides which lines a peripheral reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Jack {
    Left,
    Right,
}

impl Jack {
    pub(crate) fn index(self) -> usize {
        match self {
            Jack::Left => 0,
            Jack::Right => 1,
        }
    }

    /// Port A carries the left jack in the high nibble and the right in the low.
    fn port_a(self, nibble: u8) -> u8 {
        match self {
            Jack::Left => nibble << 4,
            Jack::Right => nibble,
        }
    }

    /// This jack's four lines out of a whole port-A byte.
    fn nibble_of(self, port_a: u8) -> u8 {
        match self {
            Jack::Left => port_a >> 4,
            Jack::Right => port_a & 0x0F,
        }
    }

    /// The two TIA pots wired to this jack.
    fn pots(self) -> [usize; 2] {
        match self {
            Jack::Left => [0, 1],
            Jack::Right => [2, 3],
        }
    }
}

/// The kinds of controller the console itself can build for a jack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControllerKind {
    Unplugged,
    Joystick,
    Paddles,
    Keypad,
}

/// The controller in a jack, holding what the player is doing with it.
pub(crate) enum Controller {
    Unplugged,
    Joystick(Joystick),
    Paddles(Paddles),
    Keypad(Keypad),
}

impl Controller {
    pub(crate) fn new(kind: ControllerKind) -> Controller {
        match kind {
            ControllerKind::Unplugged => Controller::Unplugged,
            ControllerKind::Joystick => Controller::Joystick(Joystick::default()),
            ControllerKind::Paddles => Controller::Paddles(Paddles::new()),
            ControllerKind::Keypad => Controller::Keypad(Keypad::default()),
        }
    }

    pub(crate) fn kind(&self) -> ControllerKind {
        match self {
            Controller::Unplugged => ControllerKind::Unplugged,
            Controller::Joystick(_) => ControllerKind::Joystick,
            Controller::Paddles(_) => ControllerKind::Paddles,
            Controller::Keypad(_) => ControllerKind::Keypad,
        }
    }

    /// Drive every line this peripheral owns from its current state, the moment
    /// it is plugged in.
    pub(crate) fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        match self {
            Controller::Unplugged => {}
            Controller::Joystick(joystick) => joystick.connect(jack, riot, tia),
            Controller::Paddles(paddles) => paddles.connect(jack, riot, tia),
            Controller::Keypad(keypad) => keypad.drive_columns(jack, riot, tia),
        }
    }

    /// Port A's pin levels moved. Only a peripheral whose own lines are a
    /// function of them — the keypad, scanned row by row — has anything to redo.
    pub(crate) fn refresh(&self, jack: Jack, riot: &Riot, tia: &mut Tia) {
        if let Controller::Keypad(keypad) = self {
            keypad.drive_columns(jack, riot, tia);
        }
    }

    /// A control the player worked. A peripheral ignores roles it has no part
    /// for, and inputs of the wrong shape for the part it has.
    pub(crate) fn apply(
        &mut self,
        jack: Jack,
        role: ControlRole,
        input: ControlInput,
        riot: &mut Riot,
        tia: &mut Tia,
    ) {
        match self {
            Controller::Unplugged => {}
            Controller::Joystick(joystick) => joystick.apply(jack, role, input, riot, tia),
            Controller::Paddles(paddles) => paddles.apply(jack, role, input, riot, tia),
            Controller::Keypad(keypad) => keypad.apply(jack, role, input, riot, tia),
        }
    }
}

/// Everything a jack can drive, returned to the state of an empty socket: the
/// direction lines to their pull-ups, the trigger released, the pots open.
pub(crate) fn release_jack(jack: Jack, riot: &mut Riot, tia: &mut Tia) {
    riot.set_pin_a(jack.port_a(0xF), true);
    tia.set_trigger(jack.index(), false);
    for pot in jack.pots() {
        tia.disconnect_pot(pot);
    }
}

/// The direction lines within a jack's nibble; a pressed direction pulls its
/// line low.
fn direction(role: ControlRole) -> Option<u8> {
    match role {
        ControlRole::Right => Some(0x8),
        ControlRole::Left => Some(0x4),
        ControlRole::Down => Some(0x2),
        ControlRole::Up => Some(0x1),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct Joystick {
    /// Directions currently held, as the jack-nibble bits they pull low.
    held: u8,
    fire: bool,
}

impl Joystick {
    fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        riot.set_pin_a(jack.port_a(!self.held & 0xF), true);
        riot.set_pin_a(jack.port_a(self.held), false);
        tia.set_trigger(jack.index(), self.fire);
    }

    fn apply(
        &mut self,
        jack: Jack,
        role: ControlRole,
        input: ControlInput,
        riot: &mut Riot,
        tia: &mut Tia,
    ) {
        let ControlInput::Digital(pressed) = input else {
            return;
        };
        if let Some(bit) = direction(role) {
            if pressed {
                self.held |= bit;
            } else {
                self.held &= !bit;
            }
            riot.set_pin_a(jack.port_a(bit), !pressed);
        } else if role == ControlRole::Action(0) {
            self.fire = pressed;
            tia.set_trigger(jack.index(), pressed);
        }
    }
}

/// The pair of paddles one jack carries. Their buttons have no lines of their
/// own: each shares one of the jack's direction lines, so pressing one reads
/// as that direction on SWCHA.
pub(crate) struct Paddles {
    knobs: [f32; 2],
    buttons: [bool; 2],
}

/// Paddle 0's button sits on the jack's Right line, paddle 1's on its Left.
fn button_line(paddle: usize) -> u8 {
    0x8 >> paddle
}

impl Paddles {
    fn new() -> Paddles {
        Paddles {
            knobs: [0.5; 2],
            buttons: [false; 2],
        }
    }

    fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        for (paddle, pot) in jack.pots().into_iter().enumerate() {
            tia.connect_pot(pot, self.knobs[paddle]);
            riot.set_pin_a(jack.port_a(button_line(paddle)), !self.buttons[paddle]);
        }
    }

    fn apply(
        &mut self,
        jack: Jack,
        role: ControlRole,
        input: ControlInput,
        riot: &mut Riot,
        tia: &mut Tia,
    ) {
        match (role, input) {
            (ControlRole::Knob(paddle), ControlInput::Axis(position)) if paddle < 2 => {
                let paddle = paddle as usize;
                self.knobs[paddle] = position;
                tia.set_paddle(jack.pots()[paddle], position);
            }
            (ControlRole::Action(paddle), ControlInput::Digital(pressed)) if paddle < 2 => {
                let paddle = paddle as usize;
                self.buttons[paddle] = pressed;
                riot.set_pin_a(jack.port_a(button_line(paddle)), !pressed);
            }
            _ => {}
        }
    }
}

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
    fn drive_columns(&self, jack: Jack, riot: &Riot, tia: &mut Tia) {
        let rows = jack.nibble_of(riot.port_a_level());
        let pots = jack.pots();
        tia.drive_pot(pots[0], self.column_low(0, rows));
        tia.drive_pot(pots[1], self.column_low(1, rows));
        tia.set_trigger(jack.index(), self.column_low(2, rows));
    }

    fn apply(
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
    use crate::TvStandard;
    use crate::console::Vcs;
    use crate::tia::registers::VBLANK;

    const SWCHA: u16 = 0x0280;
    const SWACNT: u16 = 0x0281;
    const INPT0: u16 = 0x08;
    const INPT4: u16 = 0x0C;
    const INPT5: u16 = 0x0D;

    fn test_vcs() -> Vcs {
        let mut rom = vec![0xEA; 0x1000];
        rom[0xFFC] = 0x00;
        rom[0xFFD] = 0xF0;
        Vcs::new(&rom, TvStandard::Ntsc, None).unwrap()
    }

    fn press(vcs: &mut Vcs, jack: Jack, role: ControlRole) {
        vcs.set_controller_input(jack, role, ControlInput::Digital(true));
    }

    fn release(vcs: &mut Vcs, jack: Jack, role: ControlRole) {
        vcs.set_controller_input(jack, role, ControlInput::Digital(false));
    }

    #[test]
    fn both_jacks_power_on_with_a_joystick() {
        let vcs = test_vcs();
        assert_eq!(vcs.plugged(Jack::Left), ControllerKind::Joystick);
        assert_eq!(vcs.plugged(Jack::Right), ControllerKind::Joystick);
        assert_eq!(vcs.peek(SWCHA), 0xFF);
    }

    #[test]
    fn the_left_joystick_drives_the_high_nibble_and_inpt4() {
        let mut vcs = test_vcs();
        for (role, bit) in [
            (ControlRole::Right, 0x80),
            (ControlRole::Left, 0x40),
            (ControlRole::Down, 0x20),
            (ControlRole::Up, 0x10),
        ] {
            press(&mut vcs, Jack::Left, role);
            assert_eq!(vcs.peek(SWCHA), !bit, "{role:?} pulls its own line");
            release(&mut vcs, Jack::Left, role);
            assert_eq!(vcs.peek(SWCHA), 0xFF);
        }
        press(&mut vcs, Jack::Left, ControlRole::Action(0));
        assert_eq!(vcs.peek(INPT4) & 0x80, 0x00);
        assert_eq!(vcs.peek(INPT5) & 0x80, 0x80);
    }

    #[test]
    fn the_right_joystick_drives_the_low_nibble_and_inpt5() {
        let mut vcs = test_vcs();
        for (role, bit) in [
            (ControlRole::Right, 0x08),
            (ControlRole::Left, 0x04),
            (ControlRole::Down, 0x02),
            (ControlRole::Up, 0x01),
        ] {
            press(&mut vcs, Jack::Right, role);
            assert_eq!(vcs.peek(SWCHA), !bit, "{role:?} pulls its own line");
            release(&mut vcs, Jack::Right, role);
            assert_eq!(vcs.peek(SWCHA), 0xFF);
        }
        press(&mut vcs, Jack::Right, ControlRole::Action(0));
        assert_eq!(vcs.peek(INPT5) & 0x80, 0x00);
        assert_eq!(vcs.peek(INPT4) & 0x80, 0x80);
    }

    #[test]
    fn paddle_buttons_land_on_their_jacks_direction_lines() {
        let mut vcs = test_vcs();
        vcs.plug(Jack::Left, ControllerKind::Paddles);
        vcs.plug(Jack::Right, ControllerKind::Paddles);
        for (jack, paddle, bit) in [
            (Jack::Left, 0, 0x80),
            (Jack::Left, 1, 0x40),
            (Jack::Right, 0, 0x08),
            (Jack::Right, 1, 0x04),
        ] {
            press(&mut vcs, jack, ControlRole::Action(paddle));
            assert_eq!(vcs.peek(SWCHA), !bit, "{jack:?} paddle {paddle}");
            release(&mut vcs, jack, ControlRole::Action(paddle));
            assert_eq!(vcs.peek(SWCHA), 0xFF);
        }
        // A paddle pair has no trigger line of its own.
        press(&mut vcs, Jack::Left, ControlRole::Action(0));
        assert_eq!(vcs.peek(INPT4) & 0x80, 0x80);
    }

    #[test]
    fn unplugging_releases_a_held_direction() {
        let mut vcs = test_vcs();
        press(&mut vcs, Jack::Left, ControlRole::Left);
        press(&mut vcs, Jack::Left, ControlRole::Action(0));
        assert_eq!(vcs.peek(SWCHA), 0xBF);
        vcs.plug(Jack::Left, ControllerKind::Unplugged);
        assert_eq!(vcs.peek(SWCHA), 0xFF);
        assert_eq!(vcs.peek(INPT4) & 0x80, 0x80);
        // The departed joystick's state does not come back with the next one.
        vcs.plug(Jack::Left, ControllerKind::Joystick);
        assert_eq!(vcs.peek(SWCHA), 0xFF);
    }

    /// Release the pot dump and give the capacitors longer than a full-scale
    /// knob needs; only a pot with a paddle behind it ever rises.
    fn charge_pots(vcs: &mut Vcs) {
        vcs.tia.write(VBLANK, 0x80);
        vcs.step_scanline();
        vcs.tia.write(VBLANK, 0x00);
        for _ in 0..400 {
            vcs.step_scanline();
        }
    }

    #[test]
    fn a_pot_with_no_paddle_behind_it_never_charges() {
        let mut vcs = test_vcs();
        charge_pots(&mut vcs);
        assert_eq!(vcs.peek(INPT0) & 0x80, 0x00);

        vcs.plug(Jack::Left, ControllerKind::Paddles);
        vcs.set_controller_input(Jack::Left, ControlRole::Knob(0), ControlInput::Axis(0.5));
        charge_pots(&mut vcs);
        assert_eq!(vcs.peek(INPT0) & 0x80, 0x80);

        vcs.plug(Jack::Left, ControllerKind::Unplugged);
        charge_pots(&mut vcs);
        assert_eq!(vcs.peek(INPT0) & 0x80, 0x00);
    }

    const INPT1: u16 = 0x09;
    const INPT2: u16 = 0x0A;
    const INPT3: u16 = 0x0B;

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
        Vcs::new(&rom, TvStandard::Ntsc, None).unwrap()
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

    #[test]
    fn plugging_mid_frame_hands_the_lines_over_cleanly() {
        let mut vcs = test_vcs();
        press(&mut vcs, Jack::Left, ControlRole::Right);
        for _ in 0..40 {
            vcs.step_scanline();
        }
        assert_eq!(vcs.peek(SWCHA), 0x7F);

        vcs.plug(Jack::Left, ControllerKind::Paddles);
        assert_eq!(vcs.plugged(Jack::Left), ControllerKind::Paddles);
        assert_eq!(vcs.peek(SWCHA), 0xFF);
        press(&mut vcs, Jack::Left, ControlRole::Action(0));
        for _ in 0..40 {
            vcs.step_scanline();
        }
        assert_eq!(vcs.peek(SWCHA), 0x7F);
    }
}
