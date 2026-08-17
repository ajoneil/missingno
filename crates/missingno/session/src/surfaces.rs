//! The machine's input surfaces rendered as JSON — the one encoding every
//! transport publishes, so the HTTP `/ports` body and the MCP `get_ports` body
//! are the same document.

use missingno_core::ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, Provider,
};
use missingno_core::system::ControlSite;
use serde_json::{Value, json};

use crate::shared::ControlSurfaces;

/// Each port with the peripherals it accepts and the one in it now, plus the
/// console's own controls. Controls carry the site and role spelling
/// `set_control` takes.
pub fn surfaces_json(surfaces: &ControlSurfaces) -> Value {
    let ports: Vec<Value> = surfaces
        .ports
        .iter()
        .map(|port| {
            let site = ControlSite::Port(port.descriptor.port);
            let options: Vec<Value> = port
                .descriptor
                .accepts
                .iter()
                .map(|peripheral| {
                    json!({
                        "id": peripheral.id.0,
                        "label": peripheral.label,
                        "provider": match peripheral.provider {
                            Provider::Console => "console",
                            Provider::Host => "host",
                        },
                        "controls": peripheral
                            .controls
                            .iter()
                            .map(|control| control_json(site, control))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "port": port.descriptor.port.0,
                "site": site.name(),
                "label": port.descriptor.label,
                "plugged": port.plugged.map(|peripheral| peripheral.0),
                "options": options,
            })
        })
        .collect();
    json!({
        "ports": ports,
        "integrated_controls": surfaces
            .integrated
            .iter()
            .map(|control| control_json(ControlSite::Integrated, control))
            .collect::<Vec<_>>(),
        "panel_controls": surfaces
            .panel
            .iter()
            .map(panel_control_json)
            .collect::<Vec<_>>(),
    })
}

fn control_json(site: ControlSite, control: &ControlDescriptor) -> Value {
    json!({
        "site": site.name(),
        "role": control.role.name(),
        "label": control.label,
        "kind": match control.kind {
            ControlKind::Button => "button",
            ControlKind::Axis => "axis",
        },
    })
}

fn panel_control_json(control: &PanelControl) -> Value {
    let mut body = json!({
        "site": ControlSite::Panel.name(),
        "role": control.role.name(),
        "label": control.label,
        "behaviour": match control.behaviour {
            PanelBehaviour::Momentary => "momentary",
            PanelBehaviour::Toggle { .. } => "toggle",
        },
    });
    if let Some((positions, default_high)) = control.toggle()
        && let Some(object) = body.as_object_mut()
    {
        object.insert("positions".into(), json!(positions));
        object.insert("default_high".into(), json!(default_high));
    }
    body
}
