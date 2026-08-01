//! The JSON request vocabulary both transports speak: hex addresses, watch
//! terms, and control input. A transport owns how it renders an answer — the
//! HTTP server's JSON and the agent tool surface's text are different products
//! — but they take the same arguments, so they parse them the same way here.

use missingno_core::inspect::WatchTerm;
use missingno_core::ports::{PeripheralId, PortId};
use missingno_core::system::{ControlId, ControlInput, ControlRole, ControlSite};
use serde_json::Value;

use crate::shared::ControlSurfaces;

/// A hex address, with or without an `0x` prefix.
pub fn parse_hex(text: &str) -> Result<u32, String> {
    let trimmed = text
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u32::from_str_radix(trimmed, 16).map_err(|_| format!("invalid hex value: {text}"))
}

/// A named argument that may arrive as a hex string or a plain integer.
pub fn parse_hex_arg(args: &Value, name: &str) -> Result<u32, String> {
    match args.get(name) {
        Some(Value::String(text)) => parse_hex(text),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| format!("'{name}' out of range")),
        _ => Err(format!("'{name}' must be a hex string or integer")),
    }
}

pub fn parse_optional_u32(value: Option<&Value>) -> Result<Option<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| "number out of range".to_string()),
        Some(Value::String(text)) => parse_hex(text).map(Some),
        Some(_) => Err("expected a number or hex string".to_string()),
    }
}

/// Either a `terms` array or a single bare term.
pub fn parse_watch_terms(args: &Value) -> Result<Vec<WatchTerm>, String> {
    if let Some(terms) = args.get("terms").and_then(Value::as_array) {
        terms.iter().map(parse_watch_term).collect()
    } else {
        Ok(vec![parse_watch_term(args)?])
    }
}

pub fn parse_watch_term(value: &Value) -> Result<WatchTerm, String> {
    let key = value
        .get("key")
        .and_then(Value::as_str)
        .ok_or("watch term missing 'key'")?
        .to_string();
    Ok(WatchTerm {
        key,
        address: parse_optional_u32(value.get("address"))?,
        value: parse_optional_u32(value.get("value"))?,
    })
}

/// A control change: where the control sits, what it does, and either an axis
/// position or a digital state.
pub fn parse_control(args: &Value) -> Result<(ControlId, ControlInput), String> {
    let site = args
        .get("site")
        .and_then(Value::as_str)
        .and_then(ControlSite::parse)
        .ok_or("'site' must be one of: integrated, panel, port0, port1")?;
    let role = args
        .get("role")
        .and_then(Value::as_str)
        .and_then(ControlRole::parse)
        .ok_or("'role' must be a control role, e.g. start, action0, up, knob0, toggle1")?;
    let input = match args.get("axis").and_then(Value::as_f64) {
        Some(axis) => ControlInput::Axis(axis as f32),
        None => ControlInput::Digital(
            args.get("pressed")
                .and_then(Value::as_bool)
                .ok_or("provide 'pressed' (bool) or 'axis' (0.0-1.0)")?,
        ),
    };
    Ok((ControlId { site, role }, input))
}

/// A plug request, resolved against the machine's own ports: which jack, and
/// which of the peripherals it accepts — named by id or by label.
pub fn parse_plug(
    surfaces: &ControlSurfaces,
    args: &Value,
) -> Result<(PortId, PeripheralId), String> {
    let known_ports = || {
        surfaces
            .ports
            .iter()
            .map(|port| format!("port{}", port.descriptor.port.0))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let wanted = match args.get("port") {
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(PortId)
            .ok_or_else(|| "'port' out of range".to_string())?,
        Some(Value::String(text)) => match ControlSite::parse(text.trim()) {
            Some(ControlSite::Port(port)) => port,
            // A bare number is the same jack, for a caller passing it as text.
            _ => text
                .trim()
                .parse()
                .map(PortId)
                .map_err(|_| format!("'port' must be one of: {}", known_ports()))?,
        },
        _ => return Err(format!("'port' must be one of: {}", known_ports())),
    };
    let port = surfaces
        .ports
        .iter()
        .find(|port| port.descriptor.port == wanted)
        .ok_or_else(|| {
            format!(
                "this console has no port{}; it has: {}",
                wanted.0,
                known_ports()
            )
        })?;

    let options = || {
        port.descriptor
            .accepts
            .iter()
            .map(|peripheral| format!("{} ({})", peripheral.label, peripheral.id.0))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let named = args
        .get("peripheral")
        .ok_or_else(|| format!("'peripheral' must be one of: {}", options()))?;
    let matches = |peripheral: &missingno_core::ports::PeripheralDescriptor| match named {
        Value::Number(number) => number.as_u64() == Some(u64::from(peripheral.id.0)),
        Value::String(text) => {
            let text = text.trim();
            peripheral.label.eq_ignore_ascii_case(text) || text.parse() == Ok(peripheral.id.0)
        }
        _ => false,
    };
    let peripheral = port
        .descriptor
        .accepts
        .iter()
        .find(|peripheral| matches(peripheral))
        .ok_or_else(|| format!("'peripheral' must be one of: {}", options()))?;
    Ok((port.descriptor.port, peripheral.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_hex_accepts_prefixes_and_plain() {
        assert_eq!(parse_hex("ff40").unwrap(), 0xff40);
        assert_eq!(parse_hex("0xFF40").unwrap(), 0xff40);
        assert!(parse_hex("nope").is_err());
    }

    #[test]
    fn parse_hex_arg_takes_string_or_number() {
        assert_eq!(parse_hex_arg(&json!({ "a": "0x20" }), "a").unwrap(), 0x20);
        assert_eq!(parse_hex_arg(&json!({ "a": 32 }), "a").unwrap(), 32);
        assert!(parse_hex_arg(&json!({}), "a").is_err());
    }

    #[test]
    fn watch_terms_take_a_list_or_a_bare_term() {
        let single = parse_watch_terms(&json!({ "key": "ly", "value": "90" })).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].value, Some(0x90));

        let list =
            parse_watch_terms(&json!({ "terms": [{ "key": "ly" }, { "key": "sp" }] })).unwrap();
        assert_eq!(list.len(), 2);
        assert!(parse_watch_terms(&json!({ "value": 3 })).is_err());
    }

    #[test]
    fn control_takes_an_axis_or_a_digital_state() {
        let (id, input) =
            parse_control(&json!({ "site": "port0", "role": "action1", "pressed": true })).unwrap();
        assert_eq!(
            id,
            ControlId::port(missingno_core::ports::PortId(0), ControlRole::Action(1))
        );
        assert_eq!(input, ControlInput::Digital(true));

        let (_, axis) =
            parse_control(&json!({ "site": "port0", "role": "knob0", "axis": 0.5 })).unwrap();
        assert_eq!(axis, ControlInput::Axis(0.5));

        assert!(parse_control(&json!({ "site": "integrated", "role": "start" })).is_err());
        assert!(parse_control(&json!({ "pressed": true })).is_err());
    }

    /// One jack taking nothing or a paddle pair, as a console's ports report.
    fn surfaces() -> ControlSurfaces {
        use missingno_core::ports::{
            ControlDescriptor, ControlKind, PeripheralDescriptor, PortDescriptor, Provider,
        };

        static KNOB: &[ControlDescriptor] = &[ControlDescriptor {
            role: ControlRole::Knob(0),
            label: "Paddle 1 Knob",
            kind: ControlKind::Axis,
        }];
        static ACCEPTS: &[PeripheralDescriptor] = &[
            PeripheralDescriptor {
                id: PeripheralId(0),
                label: "Unplugged",
                provider: Provider::Console,
                controls: &[],
            },
            PeripheralDescriptor {
                id: PeripheralId(2),
                label: "Paddles",
                provider: Provider::Console,
                controls: KNOB,
            },
        ];
        static PORTS: &[PortDescriptor] = &[PortDescriptor {
            port: PortId(0),
            label: "Left controller",
            accepts: ACCEPTS,
        }];
        ControlSurfaces {
            ports: PORTS
                .iter()
                .map(|descriptor| crate::shared::PluggedPort {
                    descriptor: *descriptor,
                    plugged: Some(PeripheralId(0)),
                })
                .collect(),
            integrated: &[],
            panel: &[],
        }
    }

    #[test]
    fn plug_names_a_port_and_a_peripheral_either_way() {
        let surfaces = surfaces();
        let expected = (PortId(0), PeripheralId(2));
        assert_eq!(
            parse_plug(&surfaces, &json!({ "port": 0, "peripheral": 2 })).unwrap(),
            expected
        );
        assert_eq!(
            parse_plug(
                &surfaces,
                &json!({ "port": "port0", "peripheral": "paddles" })
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn plug_refuses_a_port_or_peripheral_the_console_lacks() {
        let surfaces = surfaces();
        assert!(parse_plug(&surfaces, &json!({ "port": 1, "peripheral": 2 })).is_err());
        assert!(parse_plug(&surfaces, &json!({ "port": 0, "peripheral": "keypad" })).is_err());
        assert!(parse_plug(&surfaces, &json!({ "peripheral": 2 })).is_err());
    }
}
