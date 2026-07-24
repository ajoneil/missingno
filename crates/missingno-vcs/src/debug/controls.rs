//! The family's reading of the shared control ids: the standard pad's
//! directions and fire, the console's latching switches, and paddle 0's knob.

use missingno_core::system::{ConsoleSwitch, ControlId, ControlInput};

use crate::console::{JoystickDirection, Vcs};

/// The latching console switches, driven through control ids past the
/// paddle (id 8). Positions and defaults match the RIOT's SWCHB state.
pub const CONSOLE_SWITCHES: [ConsoleSwitch; 3] = [
    ConsoleSwitch {
        control: ControlId(9),
        label: "Left Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(10),
        label: "Right Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(11),
        label: "TV Type",
        positions: ["B•W", "Color"],
        default_high: true,
    },
];

/// Paddle 0's knob rides the first analog control id.
pub const PADDLE_CONTROL: ControlId = ControlId(8);

/// The standard pad maps onto the joystick and fire, Start/Select work the
/// console switches, and the paddle takes the axis.
pub(super) fn apply_control(vcs: &mut Vcs, control: ControlId, input: ControlInput) {
    match input {
        ControlInput::Digital(pressed) => {
            let direction = match control.0 {
                0 => return vcs.set_console_reset(pressed),
                1 => return vcs.set_console_select(pressed),
                2 | 3 => return vcs.set_fire(pressed),
                4 => JoystickDirection::Up,
                5 => JoystickDirection::Down,
                6 => JoystickDirection::Left,
                7 => JoystickDirection::Right,
                // Latching console switches carry their level, not a press.
                9 => return vcs.set_difficulty(0, pressed),
                10 => return vcs.set_difficulty(1, pressed),
                11 => return vcs.set_color_mode(pressed),
                _ => return,
            };
            vcs.set_joystick(direction, pressed);
        }
        ControlInput::Axis(value) => {
            if control == PADDLE_CONTROL {
                vcs.set_paddle(0, value);
            }
        }
    }
}
