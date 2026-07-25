//! The family's controls: the console panel's buttons and switches, and the
//! controllers in its two jacks.

use missingno_core::ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, PeripheralDescriptor,
    PeripheralId, PlugError, PortDescriptor, PortId, Provider,
};
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};

use crate::console::{JoystickDirection, Vcs};

/// The console's own buttons and switches. Positions and defaults match the
/// RIOT's SWCHB state.
pub const PANEL_CONTROLS: &[PanelControl] = &[
    PanelControl {
        role: ControlRole::Reset,
        label: "Reset",
        behaviour: PanelBehaviour::Momentary,
    },
    PanelControl {
        role: ControlRole::Select,
        label: "Select",
        behaviour: PanelBehaviour::Momentary,
    },
    PanelControl {
        role: ControlRole::Toggle(0),
        label: "Left Difficulty",
        behaviour: PanelBehaviour::Toggle {
            positions: ["B", "A"],
            default_high: false,
        },
    },
    PanelControl {
        role: ControlRole::Toggle(1),
        label: "Right Difficulty",
        behaviour: PanelBehaviour::Toggle {
            positions: ["B", "A"],
            default_high: false,
        },
    },
    PanelControl {
        role: ControlRole::Toggle(2),
        label: "TV Type",
        behaviour: PanelBehaviour::Toggle {
            positions: ["B•W", "Color"],
            default_high: true,
        },
    },
];

pub const LEFT_PORT: PortId = PortId(0);
pub const RIGHT_PORT: PortId = PortId(1);

pub const UNPLUGGED: PeripheralId = PeripheralId(0);
pub const JOYSTICK: PeripheralId = PeripheralId(1);
pub const PADDLES: PeripheralId = PeripheralId(2);

const JOYSTICK_CONTROLS: &[ControlDescriptor] = &[
    button(ControlRole::Action(0), "Fire"),
    button(ControlRole::Up, "Up"),
    button(ControlRole::Down, "Down"),
    button(ControlRole::Left, "Left"),
    button(ControlRole::Right, "Right"),
];

/// A pair of paddles shares one jack: two knobs and two buttons.
const PADDLE_CONTROLS: &[ControlDescriptor] = &[
    ControlDescriptor {
        role: ControlRole::Knob(0),
        label: "Paddle 1 Knob",
        kind: ControlKind::Axis,
    },
    button(ControlRole::Action(0), "Paddle 1 Button"),
    ControlDescriptor {
        role: ControlRole::Knob(1),
        label: "Paddle 2 Knob",
        kind: ControlKind::Axis,
    },
    button(ControlRole::Action(1), "Paddle 2 Button"),
];

const fn button(role: ControlRole, label: &'static str) -> ControlDescriptor {
    ControlDescriptor {
        role,
        label,
        kind: ControlKind::Button,
    }
}

const CONTROLLERS: &[PeripheralDescriptor] = &[
    PeripheralDescriptor {
        id: UNPLUGGED,
        label: "Unplugged",
        provider: Provider::Console,
        controls: &[],
    },
    PeripheralDescriptor {
        id: JOYSTICK,
        label: "Joystick",
        provider: Provider::Console,
        controls: JOYSTICK_CONTROLS,
    },
    PeripheralDescriptor {
        id: PADDLES,
        label: "Paddles",
        provider: Provider::Console,
        controls: PADDLE_CONTROLS,
    },
];

pub const PORTS: &[PortDescriptor] = &[
    PortDescriptor {
        port: LEFT_PORT,
        label: "Left controller",
        accepts: CONTROLLERS,
    },
    PortDescriptor {
        port: RIGHT_PORT,
        label: "Right controller",
        accepts: CONTROLLERS,
    },
];

/// Both jacks are wired to a joystick; swapping in a paddle pair needs the
/// per-port peripheral model.
pub(super) fn plugged(port: PortId) -> Option<PeripheralId> {
    PORTS.iter().any(|p| p.port == port).then_some(JOYSTICK)
}

pub(super) fn plug(port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
    if !PORTS.iter().any(|p| p.port == port) {
        return Err(PlugError::UnknownPort);
    }
    if peripheral == JOYSTICK {
        Ok(())
    } else {
        Err(PlugError::NotAccepted)
    }
}

/// The panel drives the RIOT's console lines; the left jack drives the
/// joystick, its trigger, and pot 0. The right jack is not wired yet.
pub(super) fn apply_control(vcs: &mut Vcs, control: ControlId, input: ControlInput) {
    match input {
        ControlInput::Digital(pressed) => match (control.site, control.role) {
            (ControlSite::Panel, ControlRole::Reset) => vcs.set_console_reset(pressed),
            (ControlSite::Panel, ControlRole::Select) => vcs.set_console_select(pressed),
            (ControlSite::Panel, ControlRole::Toggle(switch @ (0 | 1))) => {
                vcs.set_difficulty(switch as usize, pressed)
            }
            (ControlSite::Panel, ControlRole::Toggle(2)) => vcs.set_color_mode(pressed),
            (ControlSite::Port(LEFT_PORT), role) => match role {
                ControlRole::Action(0) => vcs.set_fire(pressed),
                ControlRole::Up => vcs.set_joystick(JoystickDirection::Up, pressed),
                ControlRole::Down => vcs.set_joystick(JoystickDirection::Down, pressed),
                ControlRole::Left => vcs.set_joystick(JoystickDirection::Left, pressed),
                ControlRole::Right => vcs.set_joystick(JoystickDirection::Right, pressed),
                _ => {}
            },
            _ => {}
        },
        ControlInput::Axis(value) => {
            if control == ControlId::port(LEFT_PORT, ControlRole::Knob(0)) {
                vcs.set_paddle(0, value);
            }
        }
    }
}
