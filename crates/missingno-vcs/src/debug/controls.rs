//! The family's controls: the console panel's buttons and switches, and the
//! controllers in its two jacks.

use missingno_core::ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, PeripheralDescriptor,
    PeripheralId, PlugError, PortDescriptor, PortId, Provider,
};
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};

use crate::console::Vcs;
use crate::controllers::{ControllerKind, Jack};

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
pub const KEYPAD: PeripheralId = PeripheralId(3);

const JOYSTICK_CONTROLS: &[ControlDescriptor] = &[
    button(ControlRole::Action(0), "Fire"),
    button(ControlRole::Up, "Up"),
    button(ControlRole::Down, "Down"),
    button(ControlRole::Left, "Left"),
    button(ControlRole::Right, "Right"),
];

/// One paddle of the pair a jack carries; the second stays off the frontend's
/// surfaces by choice, though the console reads it.
const PADDLE_CONTROLS: &[ControlDescriptor] = &[
    ControlDescriptor {
        role: ControlRole::Knob(0),
        label: "Paddle Knob",
        kind: ControlKind::Axis,
    },
    button(ControlRole::Action(0), "Paddle Button"),
];

/// The keyboard controller's 12 keys, row-major from its top left.
const KEYPAD_CONTROLS: &[ControlDescriptor] = &[
    button(ControlRole::Key(0), "1"),
    button(ControlRole::Key(1), "2"),
    button(ControlRole::Key(2), "3"),
    button(ControlRole::Key(3), "4"),
    button(ControlRole::Key(4), "5"),
    button(ControlRole::Key(5), "6"),
    button(ControlRole::Key(6), "7"),
    button(ControlRole::Key(7), "8"),
    button(ControlRole::Key(8), "9"),
    button(ControlRole::Key(9), "*"),
    button(ControlRole::Key(10), "0"),
    button(ControlRole::Key(11), "#"),
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
    PeripheralDescriptor {
        id: KEYPAD,
        label: "Keypad",
        provider: Provider::Console,
        controls: KEYPAD_CONTROLS,
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

/// The jack a port names, if the console has one.
fn jack(port: PortId) -> Option<Jack> {
    if port == LEFT_PORT {
        Some(Jack::Left)
    } else if port == RIGHT_PORT {
        Some(Jack::Right)
    } else {
        None
    }
}

fn peripheral_id(kind: ControllerKind) -> PeripheralId {
    match kind {
        ControllerKind::Unplugged => UNPLUGGED,
        ControllerKind::Joystick => JOYSTICK,
        ControllerKind::Paddles => PADDLES,
        ControllerKind::Keypad => KEYPAD,
    }
}

fn controller_kind(peripheral: PeripheralId) -> Option<ControllerKind> {
    if peripheral == UNPLUGGED {
        Some(ControllerKind::Unplugged)
    } else if peripheral == JOYSTICK {
        Some(ControllerKind::Joystick)
    } else if peripheral == PADDLES {
        Some(ControllerKind::Paddles)
    } else if peripheral == KEYPAD {
        Some(ControllerKind::Keypad)
    } else {
        None
    }
}

pub(super) fn plugged(vcs: &Vcs, port: PortId) -> Option<PeripheralId> {
    Some(peripheral_id(vcs.plugged(jack(port)?)))
}

pub(super) fn plug(vcs: &mut Vcs, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
    let jack = jack(port).ok_or(PlugError::UnknownPort)?;
    vcs.plug(
        jack,
        controller_kind(peripheral).ok_or(PlugError::NotAccepted)?,
    );
    Ok(())
}

/// The panel drives the RIOT's console lines directly; a port's roles go to
/// whatever is plugged into that jack.
pub(super) fn apply_control(vcs: &mut Vcs, control: ControlId, input: ControlInput) {
    match control.site {
        ControlSite::Panel => apply_panel(vcs, control.role, input),
        ControlSite::Port(port) => {
            if let Some(jack) = jack(port) {
                vcs.set_controller_input(jack, control.role, input);
            }
        }
        ControlSite::Integrated => {}
    }
}

fn apply_panel(vcs: &mut Vcs, role: ControlRole, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    match role {
        ControlRole::Reset => vcs.set_console_reset(pressed),
        ControlRole::Select => vcs.set_console_select(pressed),
        ControlRole::Toggle(switch @ (0 | 1)) => vcs.set_difficulty(switch as usize, pressed),
        ControlRole::Toggle(2) => vcs.set_color_mode(pressed),
        _ => {}
    }
}
