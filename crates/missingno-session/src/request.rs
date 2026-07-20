//! The JSON request vocabulary both transports speak: hex addresses, watch
//! terms, and control input. A transport owns how it renders an answer — the
//! HTTP server's JSON and the agent tool surface's text are different products
//! — but they take the same arguments, so they parse them the same way here.

use missingno_core::inspect::WatchTerm;
use missingno_core::system::{ControlId, ControlInput};
use serde_json::Value;

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

/// A control change: an id plus either an axis position or a digital state.
pub fn parse_control(args: &Value) -> Result<(ControlId, ControlInput), String> {
    let control = args
        .get("control")
        .and_then(Value::as_u64)
        .and_then(|n| u8::try_from(n).ok())
        .ok_or("'control' must be an integer 0-255")?;
    let input = match args.get("axis").and_then(Value::as_f64) {
        Some(axis) => ControlInput::Axis(axis as f32),
        None => ControlInput::Digital(
            args.get("pressed")
                .and_then(Value::as_bool)
                .ok_or("provide 'pressed' (bool) or 'axis' (0.0-1.0)")?,
        ),
    };
    Ok((ControlId(control), input))
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
        let (id, input) = parse_control(&json!({ "control": 3, "pressed": true })).unwrap();
        assert_eq!(id.0, 3);
        assert_eq!(input, ControlInput::Digital(true));

        let (_, axis) = parse_control(&json!({ "control": 0, "axis": 0.5 })).unwrap();
        assert_eq!(axis, ControlInput::Axis(0.5));

        assert!(parse_control(&json!({ "control": 0 })).is_err());
        assert!(parse_control(&json!({ "pressed": true })).is_err());
    }
}
