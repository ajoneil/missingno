//! What is plugged into the console's two controller jacks.
//!
//! A jack carries four direction lines into the RIOT's port A, a trigger line
//! into the TIA, and two paddle pots. Which of them a peripheral drives — and
//! what a player's input does to them — is the peripheral's business, so each
//! kind holds its own user-facing state and pushes line levels down as that
//! state changes. Nothing behind an empty jack drives anything: the direction
//! lines sit at their pull-ups and the pots have no charge path at all.

mod joystick;
mod keypad;
mod paddles;

use missingno_core::system::{ControlInput, ControlRole};

use crate::riot::Riot;
use crate::tia::Tia;
use joystick::Joystick;
use keypad::Keypad;
use paddles::Paddles;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Vcs;
    use crate::{DumpFit, TvStandard};

    pub(super) const SWCHA: u16 = 0x0280;
    pub(super) const SWACNT: u16 = 0x0281;
    pub(super) const INPT0: u16 = 0x08;
    pub(super) const INPT1: u16 = 0x09;
    pub(super) const INPT2: u16 = 0x0A;
    pub(super) const INPT3: u16 = 0x0B;
    pub(super) const INPT4: u16 = 0x0C;
    pub(super) const INPT5: u16 = 0x0D;

    pub(super) fn test_vcs() -> Vcs {
        let mut rom = vec![0xEA; 0x1000];
        rom[0xFFC] = 0x00;
        rom[0xFFD] = 0xF0;
        Vcs::new(&rom, TvStandard::Ntsc, None, DumpFit::Exact).unwrap()
    }

    pub(super) fn press(vcs: &mut Vcs, jack: Jack, role: ControlRole) {
        vcs.set_controller_input(jack, role, ControlInput::Digital(true));
    }

    pub(super) fn release(vcs: &mut Vcs, jack: Jack, role: ControlRole) {
        vcs.set_controller_input(jack, role, ControlInput::Digital(false));
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
