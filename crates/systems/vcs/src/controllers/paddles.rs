//! The pair of paddles one jack carries: two pots, two buttons.

use missingno_core::system::{ControlInput, ControlRole};

use super::Jack;
use crate::riot::Riot;
use crate::tia::Tia;

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
    pub(super) fn new() -> Paddles {
        Paddles {
            knobs: [0.5; 2],
            buttons: [false; 2],
        }
    }

    pub(super) fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        for (paddle, pot) in jack.pots().into_iter().enumerate() {
            tia.connect_pot(pot, self.knobs[paddle]);
            riot.set_pin_a(jack.port_a(button_line(paddle)), !self.buttons[paddle]);
        }
    }

    pub(super) fn apply(
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
    use crate::console::Vcs;
    use crate::controllers::ControllerKind;
    use crate::controllers::tests::{INPT0, INPT4, SWCHA, press, release, test_vcs};
    use crate::tia::registers::VBLANK;

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
}
