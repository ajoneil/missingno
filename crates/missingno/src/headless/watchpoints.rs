use missingno_gb::debugger::WatchCondition;
use missingno_gb::ppu::rendering::Mode;

pub(super) fn watchpoint_json(condition: &WatchCondition) -> serde_json::Value {
    match condition {
        WatchCondition::BusRead { address } => serde_json::json!({
            "type": "bus_read",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::BusWrite { address } => serde_json::json!({
            "type": "bus_write",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::DmaRead { address } => serde_json::json!({
            "type": "dma_read",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::DmaWrite { address } => serde_json::json!({
            "type": "dma_write",
            "address": format!("{address:04x}"),
        }),
        WatchCondition::Scanline(ly) => serde_json::json!({
            "type": "scanline",
            "value": ly,
        }),
        WatchCondition::PpuMode(mode) => serde_json::json!({
            "type": "ppu_mode",
            "mode": match mode {
                Mode::HorizontalBlank => "hblank",
                Mode::VerticalBlank => "vblank",
                Mode::OamScan => "oam_scan",
                Mode::Drawing => "drawing",
            },
        }),
        WatchCondition::PixelCounter(pc) => serde_json::json!({
            "type": "pixel_counter",
            "value": pc,
        }),
        WatchCondition::PpuRegister { register, value } => serde_json::json!({
            "type": "ppu_register",
            "register": format!("{register:?}"),
            "value": value,
        }),
        WatchCondition::CpuRegister { register, value } => serde_json::json!({
            "type": "cpu_register",
            "register": format!("{register:?}"),
            "value": value,
        }),
        WatchCondition::All(conditions) => serde_json::json!({
            "type": "all",
            "conditions": conditions.iter().map(watchpoint_json).collect::<Vec<_>>(),
        }),
    }
}

pub(super) fn parse_watchpoint_body(body: &str) -> Result<WatchCondition, String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    parse_watchpoint_json(&json)
}

fn parse_watchpoint_json(json: &serde_json::Value) -> Result<WatchCondition, String> {
    let typ = json["type"].as_str().ok_or("missing \"type\" field")?;
    match typ {
        "bus_read" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::BusRead { address: addr })
        }
        "bus_write" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::BusWrite { address: addr })
        }
        "dma_read" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::DmaRead { address: addr })
        }
        "dma_write" => {
            let addr = parse_hex_field(json, "address")?;
            Ok(WatchCondition::DmaWrite { address: addr })
        }
        "scanline" => {
            let value = json["value"].as_u64().ok_or("missing \"value\" field")? as u8;
            Ok(WatchCondition::Scanline(value))
        }
        "pixel_counter" => {
            let value = json["value"].as_u64().ok_or("missing \"value\" field")? as u8;
            Ok(WatchCondition::PixelCounter(value))
        }
        "ppu_mode" => {
            let mode_str = json["mode"].as_str().ok_or("missing \"mode\" field")?;
            let mode = match mode_str {
                "hblank" | "0" => Mode::HorizontalBlank,
                "vblank" | "1" => Mode::VerticalBlank,
                "oam_scan" | "2" => Mode::OamScan,
                "drawing" | "3" => Mode::Drawing,
                _ => return Err(format!("invalid mode: {mode_str}")),
            };
            Ok(WatchCondition::PpuMode(mode))
        }
        "all" => {
            let conditions = json["conditions"]
                .as_array()
                .ok_or("missing \"conditions\" array")?;
            let parsed: Result<Vec<_>, _> = conditions.iter().map(parse_watchpoint_json).collect();
            Ok(WatchCondition::All(parsed?))
        }
        other => Err(format!("unknown type: {other}")),
    }
}

fn parse_hex_field(json: &serde_json::Value, field: &str) -> Result<u16, String> {
    let s = json[field]
        .as_str()
        .ok_or(format!("missing \"{field}\" field"))?;
    u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex in \"{field}\": {s}"))
}
