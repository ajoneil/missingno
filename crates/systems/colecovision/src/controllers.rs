//! The two hand controllers and the latch that picks which half of each one
//! the buffers read. One cross-coupled pair (U24A/U24B) drives both
//! connectors' commons, so a single write selects the segment for both ports.

use missingno_core::system::ControlRole;

/// Which common the mode latch drives low.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControllerMode {
    /// Segment 0, after a write to $C0-$DF: the stick and the left button.
    Joystick,
    /// Segment 1, after a write to $80-$9F: the keypad and the right button.
    Keypad,
}

/// Every buffer input is pulled up and a closed switch reads 0.
const RELEASED: u8 = 0xFF;
/// The stick's four lines, and the keypad's four encoded lines, share D0-D3.
const LOW_NIBBLE: u8 = 0x0F;
/// The fire button on each segment: left on the joystick's, right on the
/// keypad's.
const FIRE: u8 = 0x40;

pub const UP: u8 = 0x01;
pub const RIGHT: u8 = 0x02;
pub const DOWN: u8 = 0x04;
pub const LEFT: u8 = 0x08;

/// The four lines each key's diodes leave high, in [`ControlRole::Key`] order
/// — `1`-`9`, `*`, `0`, `#` — from OS 7's decode table.
const KEY_CODES: [u8; KEYS] = [0xD, 0x7, 0xC, 0x2, 0x3, 0xE, 0x5, 0x1, 0xB, 0x9, 0xA, 0x6];
pub const KEYS: usize = 12;

/// A hand controller: the stick's four lines, the two buttons on their
/// segments, and the twelve keys the diode matrix encodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HandController {
    /// Active low, bits 0-3: Up, Right, Down, Left.
    pub lines: u8,
    pub left_fire: bool,
    pub right_fire: bool,
    /// Bit n held for [`ControlRole::Key`] n.
    pub keys: u16,
}

impl Default for HandController {
    fn default() -> Self {
        HandController {
            lines: LOW_NIBBLE,
            left_fire: false,
            right_fire: false,
            keys: 0,
        }
    }
}

impl HandController {
    /// The byte the buffer presents for the segment the latch selects. Bits
    /// 4, 5 and 7 have no line on a hand controller and read their pull-ups.
    pub fn read(&self, mode: ControllerMode) -> u8 {
        let (nibble, fire) = match mode {
            ControllerMode::Joystick => (self.lines & LOW_NIBBLE, self.left_fire),
            ControllerMode::Keypad => (self.keypad_code(), self.right_fire),
        };
        let byte = (RELEASED & !LOW_NIBBLE) | nibble;
        if fire { byte & !FIRE } else { byte }
    }

    /// Each held key's diodes pull their lines low, so two keys read the AND
    /// of their codes.
    fn keypad_code(&self) -> u8 {
        KEY_CODES
            .iter()
            .enumerate()
            .filter(|(key, _)| self.keys & (1 << key) != 0)
            .fold(LOW_NIBBLE, |code, (_, key_code)| code & key_code)
    }

    /// Set or release the switch `role` names; `false` for a role this
    /// controller has no switch for.
    pub fn apply(&mut self, role: ControlRole, pressed: bool) -> bool {
        let line = match role {
            ControlRole::Up => UP,
            ControlRole::Right => RIGHT,
            ControlRole::Down => DOWN,
            ControlRole::Left => LEFT,
            ControlRole::Action(0) => {
                self.left_fire = pressed;
                return true;
            }
            ControlRole::Action(1) => {
                self.right_fire = pressed;
                return true;
            }
            ControlRole::Key(key) if (key as usize) < KEYS => {
                let bit = 1u16 << key;
                if pressed {
                    self.keys |= bit;
                } else {
                    self.keys &= !bit;
                }
                return true;
            }
            _ => return false,
        };
        if pressed {
            self.lines &= !line;
        } else {
            self.lines |= line;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_released_controller_reads_all_ones_on_both_segments() {
        let pad = HandController::default();
        assert_eq!(pad.read(ControllerMode::Joystick), 0xFF);
        assert_eq!(pad.read(ControllerMode::Keypad), 0xFF);
    }

    #[test]
    fn each_button_reads_on_its_own_segment_only() {
        let mut pad = HandController::default();
        pad.apply(ControlRole::Action(0), true);
        assert_eq!(pad.read(ControllerMode::Joystick), 0xBF);
        assert_eq!(pad.read(ControllerMode::Keypad), 0xFF);

        let mut pad = HandController::default();
        pad.apply(ControlRole::Action(1), true);
        assert_eq!(pad.read(ControllerMode::Joystick), 0xFF);
        assert_eq!(pad.read(ControllerMode::Keypad), 0xBF);
    }

    #[test]
    fn each_key_reads_its_code_and_two_keys_their_and() {
        let codes = [0xD, 0x7, 0xC, 0x2, 0x3, 0xE, 0x5, 0x1, 0xB, 0x9, 0xA, 0x6];
        for (key, code) in codes.into_iter().enumerate() {
            let mut pad = HandController::default();
            pad.apply(ControlRole::Key(key as u8), true);
            assert_eq!(pad.read(ControllerMode::Keypad), 0xF0 | code, "key {key}");
        }
        let mut pad = HandController::default();
        pad.apply(ControlRole::Key(0), true);
        pad.apply(ControlRole::Key(1), true);
        assert_eq!(pad.read(ControllerMode::Keypad) & 0x0F, 0x5);
    }

    #[test]
    fn the_stick_lines_read_active_low() {
        let mut pad = HandController::default();
        pad.apply(ControlRole::Up, true);
        pad.apply(ControlRole::Left, true);
        assert_eq!(pad.read(ControllerMode::Joystick), 0xF6);
        pad.apply(ControlRole::Up, false);
        assert_eq!(pad.read(ControllerMode::Joystick), 0xF7);
    }
}
