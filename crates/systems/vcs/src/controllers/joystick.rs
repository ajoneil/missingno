//! The standard joystick: four direction lines and one trigger.

use missingno_core::system::{ControlInput, ControlRole};

use super::Jack;
use crate::riot::Riot;
use crate::tia::Tia;

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
    pub(super) fn connect(&self, jack: Jack, riot: &mut Riot, tia: &mut Tia) {
        riot.set_pin_a(jack.port_a(!self.held & 0xF), true);
        riot.set_pin_a(jack.port_a(self.held), false);
        tia.set_trigger(jack.index(), self.fire);
    }

    pub(super) fn apply(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::ControllerKind;
    use crate::controllers::tests::{INPT4, INPT5, SWCHA, press, release, test_vcs};

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
}
