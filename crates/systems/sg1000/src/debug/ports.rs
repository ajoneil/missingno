//! What a player can reach: the two joystick sites the multiplexer pair
//! presents, and the pause switch on the console's own shell.

use missingno_core::ports::{
    ControlDescriptor, PanelBehaviour, PanelControl, PeripheralDescriptor, PeripheralId,
    PortDescriptor, PortId, Provider,
};
use missingno_core::system::ControlRole;

use crate::console::{JOY1, JOY2};

pub(crate) const CONTROL_PAD: PeripheralId = PeripheralId(0);

/// Pause is a switch on the console itself, wired to /NMI rather than to a
/// controller line.
pub const PANEL: &[PanelControl] = &[PanelControl {
    role: ControlRole::Pause,
    label: "Pause",
    behaviour: PanelBehaviour::Momentary,
}];

const PAD_BUTTONS: &[ControlDescriptor] = &[
    ControlDescriptor::button(ControlRole::Action(0), "Button 1"),
    ControlDescriptor::button(ControlRole::Action(1), "Button 2"),
    ControlDescriptor::button(ControlRole::Up, "Up"),
    ControlDescriptor::button(ControlRole::Down, "Down"),
    ControlDescriptor::button(ControlRole::Left, "Left"),
    ControlDescriptor::button(ControlRole::Right, "Right"),
];

const fn pad_port(port: PortId, label: &'static str) -> PortDescriptor {
    PortDescriptor {
        port,
        label,
        accepts: &[PeripheralDescriptor {
            id: CONTROL_PAD,
            label: "Control pad",
            provider: Provider::Console,
            controls: PAD_BUTTONS,
        }],
    }
}

/// Both joystick sites the multiplexer pair presents. Player 1's stick is
/// wired to the board rather than to a connector, but it reads through the
/// same mux as player 2's.
pub const PORTS: &[PortDescriptor] = &[
    pad_port(JOY1, "Control pad 1"),
    pad_port(JOY2, "Control pad 2"),
];
