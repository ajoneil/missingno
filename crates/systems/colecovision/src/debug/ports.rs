//! What a player can reach: a hand controller in each connector, and the Reset
//! button on the console's own shell.

use missingno_core::ports::{
    ControlDescriptor, PanelBehaviour, PanelControl, PeripheralDescriptor, PeripheralId,
    PortDescriptor, PortId, Provider,
};
use missingno_core::system::ControlRole;

use crate::console::{PORT1, PORT2};

pub(crate) const HAND_CONTROLLER: PeripheralId = PeripheralId(0);

/// Reset pulls `CPU_RESET` low; it is not a controller line.
pub const PANEL: &[PanelControl] = &[PanelControl {
    role: ControlRole::Reset,
    label: "Reset",
    behaviour: PanelBehaviour::Momentary,
}];

/// The stick, the two side buttons, and the keypad row-major from its top
/// left.
const HAND_CONTROLS: &[ControlDescriptor] = &[
    ControlDescriptor::button(ControlRole::Up, "Up"),
    ControlDescriptor::button(ControlRole::Down, "Down"),
    ControlDescriptor::button(ControlRole::Left, "Left"),
    ControlDescriptor::button(ControlRole::Right, "Right"),
    ControlDescriptor::button(ControlRole::Action(0), "Left fire"),
    ControlDescriptor::button(ControlRole::Action(1), "Right fire"),
    ControlDescriptor::button(ControlRole::Key(0), "1"),
    ControlDescriptor::button(ControlRole::Key(1), "2"),
    ControlDescriptor::button(ControlRole::Key(2), "3"),
    ControlDescriptor::button(ControlRole::Key(3), "4"),
    ControlDescriptor::button(ControlRole::Key(4), "5"),
    ControlDescriptor::button(ControlRole::Key(5), "6"),
    ControlDescriptor::button(ControlRole::Key(6), "7"),
    ControlDescriptor::button(ControlRole::Key(7), "8"),
    ControlDescriptor::button(ControlRole::Key(8), "9"),
    ControlDescriptor::button(ControlRole::Key(9), "*"),
    ControlDescriptor::button(ControlRole::Key(10), "0"),
    ControlDescriptor::button(ControlRole::Key(11), "#"),
];

const fn controller_port(port: PortId, label: &'static str) -> PortDescriptor {
    PortDescriptor {
        port,
        label,
        accepts: &[PeripheralDescriptor {
            id: HAND_CONTROLLER,
            label: "Hand controller",
            provider: Provider::Console,
            controls: HAND_CONTROLS,
        }],
    }
}

/// J5 and J6, in the order the buffers' A1 select reads them.
pub const PORTS: &[PortDescriptor] = &[
    controller_port(PORT1, "Controller 1"),
    controller_port(PORT2, "Controller 2"),
];
