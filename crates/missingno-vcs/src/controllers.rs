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
}

/// The controller in a jack, holding what the player is doing with it.
pub(crate) enum Controller {
    Unplugged,
    Joystick(Joystick),
    Paddles(Paddles),
}

impl Controller {
    pub(crate) fn new(kind: ControllerKind) -> Controller {
        match kind {
            ControllerKind::Unplugged => Controller::Unplugged,
            ControllerKind::Joystick => Controller::Joystick(Joystick::default()),
            ControllerKind::Paddles => Controller::Paddles(Paddles::new()),
        }
    }

    pub(crate) fn kind(&self) -> ControllerKind {
        match self {
            Controller::Unplugged => ControllerKind::Unplugged,
            Controller::Joystick(_) => ControllerKind::Joystick,
            Controller::Paddles(_) => ControllerKind::Paddles,
        }
    }

    /// Drive every line this peripheral owns from its current state, the moment
    /// it is plugged in.
    pub(crate) fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        match self {
            Controller::Unplugged => {}
            Controller::Joystick(joystick) => joystick.connect(jack, riot, tia),
            Controller::Paddles(paddles) => paddles.connect(jack, riot, tia),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TvStandard;
    use crate::console::Vcs;
    use crate::tia::registers::VBLANK;

    const SWCHA: u16 = 0x0280;
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
