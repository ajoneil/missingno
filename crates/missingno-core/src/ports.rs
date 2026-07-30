//! Ports and the peripherals plugged into them: the structure behind a
//! console's controls.
//!
//! A console's built-in controls — the Game Boy's buttons, the VCS panel's
//! Reset and Select — are wired to the machine and always present. Everything
//! else arrives through a port: a controller jack, the Game Boy's link socket.
//! What a port carries is a runtime choice, and the peripheral chosen there
//! decides which controls exist. Descriptors state that structure, naming each
//! control by the [`ControlRole`](crate::system::ControlRole) it plays; the
//! surface listing it supplies the site.

use crate::system::ControlRole;

/// A physical connection point on the console: a controller jack, the link port.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct PortId(pub u8);

/// A family-interpreted peripheral kind id, unique within the family.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct PeripheralId(pub u8);

/// Who constructs the peripheral when it is plugged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    /// The core builds it on [`plug`](crate::system::SystemConsole::plug) — a
    /// joystick, a paddle pair, a disconnected port.
    Console,
    /// The host must construct and attach it (a printer with a file sink, a
    /// link cable over a socket); `plug` alone cannot select it.
    Host,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlKind {
    Button,
    /// A continuous position, driven by [`ControlInput::Axis`](crate::system::ControlInput::Axis).
    Axis,
}

/// One control a surface offers. The site comes from the surface listing it,
/// so a descriptor names only the role.
#[derive(Clone, Copy, Debug)]
pub struct ControlDescriptor {
    pub role: ControlRole,
    pub label: &'static str,
    pub kind: ControlKind,
}

/// A control mounted on the console shell rather than on a controller.
#[derive(Clone, Copy, Debug)]
pub struct PanelControl {
    pub role: ControlRole,
    pub label: &'static str,
    pub behaviour: PanelBehaviour,
}

/// How a panel control is worked.
#[derive(Clone, Copy, Debug)]
pub enum PanelBehaviour {
    /// Pressed and released, like the VCS's Reset.
    Momentary,
    /// Left in a position the user flips; the new level travels through
    /// `set_control` as [`ControlInput::Digital`](crate::system::ControlInput::Digital).
    Toggle {
        /// Position names for the two levels, `[low, high]`.
        positions: [&'static str; 2],
        /// The power-on level, matching the core's default switch state.
        default_high: bool,
    },
}

impl PanelControl {
    /// The switch positions and power-on level, for a control the user leaves
    /// in a position rather than pressing.
    pub fn toggle(&self) -> Option<([&'static str; 2], bool)> {
        match self.behaviour {
            PanelBehaviour::Toggle {
                positions,
                default_high,
            } => Some((positions, default_high)),
            PanelBehaviour::Momentary => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PeripheralDescriptor {
    pub id: PeripheralId,
    pub label: &'static str,
    pub provider: Provider,
    /// The controls this peripheral exposes while plugged. Ids collide only
    /// between peripherals of one port, which are mutually exclusive.
    pub controls: &'static [ControlDescriptor],
}

#[derive(Clone, Copy, Debug)]
pub struct PortDescriptor {
    pub port: PortId,
    pub label: &'static str,
    pub accepts: &'static [PeripheralDescriptor],
}

/// Why a peripheral could not be plugged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlugError {
    /// The console has no such port.
    UnknownPort,
    /// The peripheral kind is not in this port's accepts list.
    NotAccepted,
    /// A [`Provider::Host`] kind: it needs a host-side attach, not a plug.
    HostProvided,
}

impl std::fmt::Display for PlugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PlugError::UnknownPort => "this console has no such port",
            PlugError::NotAccepted => "that port does not accept this peripheral",
            PlugError::HostProvided => {
                "this peripheral is supplied by the host and cannot be plugged from the seam"
            }
        })
    }
}

impl std::error::Error for PlugError {}
