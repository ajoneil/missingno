//! The HTTP transport over a [`Session`]. This module only routes requests and
//! encodes JSON — every debugger operation is a `Session` call. The endpoints
//! are all generic: they render whatever the seam schema reports, so the same
//! routes serve any core.

use missingno_core::graphics::{
    GraphicsView, MapEntry, NamedPalette, Object, ObjectTable, PaletteSet, TileAtlas, TileMap,
    Viewport,
};
use missingno_core::inspect::{
    BitTable, PairMatrix, PixelStrip, Pointer, Register, RegisterPair, Row, Section, SectionBlock,
    SwatchRow, Sweep, Tone, ValueStyle, Watch,
};
use missingno_core::waveform::ChannelWave;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use missingno_session::request::{parse_control, parse_hex, parse_plug, parse_watch_terms};
use missingno_session::session::{DisasmLine, Session, StopReason};
use missingno_session::shared::{SessionHandle, SharedSession};
use missingno_session::surfaces::surfaces_json;

/// Cap on a single `/memory` read, so a bad length can't allocate unbounded.
const MAX_MEMORY_LEN: u32 = 0x1000;
/// Default and cap for a `/disassembly` window.
const DEFAULT_DISASM_COUNT: usize = 16;
const MAX_DISASM_COUNT: usize = 256;
/// Cap on sub-instruction ticks run by a single `/step-tick`.
const MAX_TICK_COUNT: usize = 1_000_000;

/// Serve `session` on `127.0.0.1:<port>` until the process is killed. This
/// transport is a client of the shared session: each request routes through the
/// handle onto the session thread, where the same generic [`Session`] seam
/// answers it, so the same routes serve any core. Accepts anything that becomes
/// a [`SharedSession`] — a `SharedSession` directly, or a bare `Session` a
/// caller built itself.
pub fn serve(session: impl Into<SharedSession>, port: u16) -> std::io::Result<()> {
    let session = session.into();
    let client = session.handle();
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address)
        .map_err(|e| std::io::Error::other(format!("failed to bind {address}: {e}")))?;
    eprintln!(
        "headless debugger ready: {}",
        client.with_session(|session| session.game_title())
    );
    eprintln!("listening on http://{address}");
    for request in server.incoming_requests() {
        // The session-level routes are answered by the command queue, so they
        // must not be wrapped in a job that runs against the debugger.
        if let Some(request) = session_route(request, &client) {
            client.with_session(move |session| handle(request, session));
        }
    }
    Ok(())
}

/// Answer the routes the session's command queue owns — free-running control,
/// input, and recording capture — handing back any request they do not claim.
fn session_route(mut request: Request, client: &SessionHandle) -> Option<Request> {
    let path = split_url(request.url()).0.to_string();
    match (request.method().clone(), path.as_str()) {
        (Method::Get, "/run") => respond_json(request, run_state_json(client)),
        (Method::Post, "/run") => {
            client.run();
            respond_json(request, run_state_json(client));
        }
        (Method::Post, "/pause") => {
            client.pause();
            respond_json(request, run_state_json(client));
        }
        (Method::Post, "/control") => {
            let outcome = read_json_body(&mut request)
                .and_then(|body| parse_control(&body))
                .map(|(control, input)| {
                    client.set_control(control, input);
                    json!({ "site": control.site.name(), "role": control.role.name() })
                });
            respond_result(request, outcome);
        }
        (Method::Get, "/ports") => {
            respond_json(request, surfaces_json(&client.control_surfaces()));
        }
        (Method::Post, "/plug") => {
            let surfaces = client.control_surfaces();
            let outcome = read_json_body(&mut request)
                .and_then(|body| parse_plug(&surfaces, &body))
                .and_then(|(port, peripheral)| {
                    client.plug(port, peripheral)?;
                    Ok(json!({ "port": port.0, "peripheral": peripheral.0 }))
                });
            respond_result(request, outcome);
        }
        (Method::Post, "/state/save") => path_command(request, |path| {
            client.save_state(path.clone().into())?;
            Ok(json!({ "saved": path }))
        }),
        (Method::Post, "/state/load") => path_command(request, |path| {
            client.load_state(path.into())?;
            Ok(loaded_state_json(client))
        }),
        (Method::Post, "/recording/start") => path_command(request, |path| {
            client.start_recording(path.clone().into())?;
            Ok(json!({ "recording": path }))
        }),
        (Method::Post, "/recording/stop") => {
            respond_result(
                request,
                client.stop_recording().map(|()| run_state_json(client)),
            );
        }
        (Method::Post, "/recording/play") => path_command(request, |path| {
            client.play_recording(path.clone().into())?;
            Ok(json!({ "playing": path }))
        }),
        _ => return Some(request),
    }
    None
}

/// Run a command over the `{ "path": … }` body a request carries, answering with
/// the JSON it produced or a 400 naming the failure.
fn path_command(mut request: Request, act: impl FnOnce(String) -> Result<Value, String>) {
    let outcome = read_path_body(&mut request).and_then(act);
    respond_result(request, outcome);
}

/// Answer a fallible command: its JSON body, or a 400 naming the failure.
fn respond_result(request: Request, outcome: Result<Value, String>) {
    match outcome {
        Ok(body) => respond_json(request, body),
        Err(message) => respond_error(request, 400, &message),
    }
}

/// The post-load view: the debugger's status where there is one, and the run
/// state for a session hosting a plain console.
fn loaded_state_json(client: &SessionHandle) -> Value {
    if client.is_debugger() {
        client.with_session(|session| status_json(session))
    } else {
        run_state_json(client)
    }
}

fn run_state_json(client: &SessionHandle) -> Value {
    json!({ "running": client.is_running(), "recording": client.is_recording() })
}

/// Read a request body as JSON, so the shared argument parsers can take it.
fn read_json_body(request: &mut Request) -> Result<Value, String> {
    let mut body = String::new();
    request
        .as_reader()
        .read_to_string(&mut body)
        .map_err(|_| "could not read request body".to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))
}

/// Split a request target into its path and query string. Both routing passes
/// match on the path, so a query string never changes which route claims a
/// request.
fn split_url(url: &str) -> (&str, &str) {
    url.split_once('?').unwrap_or((url, ""))
}

fn handle(request: Request, session: &mut Session) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let (path, query) = split_url(&url);

    match (&method, path) {
        (Method::Get, "/status") => respond_json(request, status_json(session)),
        (Method::Get, "/registers") => respond_json(request, registers_json(session)),
        (Method::Get, "/sections") => respond_json(request, sections_json(session)),
        (Method::Get, "/regions") => respond_json(request, regions_json(session)),
        (Method::Get, "/breakpoints") => respond_json(request, breakpoints_json(session)),
        (Method::Get, "/watchables") => respond_json(request, watchables_json(session)),
        (Method::Get, "/watches") => respond_json(request, watches_json(session)),
        (Method::Get, "/symbols") => respond_json(request, symbols_json(session)),
        (Method::Get, "/disassembly") => disassembly(request, session, query),
        (Method::Get, "/frame/bitmap") => frame_bitmap(request, session),
        (Method::Get, "/frame/raw") => respond_json(request, frame_raw_json(session)),
        (Method::Get, "/waveforms") => respond_json(request, waveforms_json(session)),
        (Method::Get, "/graphics") => respond_json(request, graphics_json(session)),

        (Method::Post, "/step") => step(request, session, Session::step),
        (Method::Post, "/step-over") => step(request, session, Session::step_over),
        (Method::Post, "/step-frame") => step(request, session, Session::step_frame),
        (Method::Post, "/step-tick") => step_tick(request, session, query),
        (Method::Post, "/recording/replay") => recording_replay(request, session),
        (Method::Post, "/reset") => {
            session.reset();
            respond_json(request, status_json(session));
        }
        (Method::Post, "/waveforms/capture") => capture_edit(request, session, Capture::Waves),
        (Method::Post, "/graphics/capture") => capture_edit(request, session, Capture::Graphics),

        (Method::Put, "/watches") => watch_edit(request, session, WatchEdit::Add),
        (Method::Delete, "/watches") => watch_edit(request, session, WatchEdit::Remove),

        _ if path.starts_with("/memory/") => memory(request, session, &method, path),
        _ if path.starts_with("/breakpoints/") => breakpoint_edit(request, session, &method, path),

        _ => respond_error(request, 404, "not found"),
    }
}

fn status_json(session: &Session) -> Value {
    json!({
        "pc": format!("{:x}", session.pc()),
        "frame": session.frame(),
        "title": session.game_title(),
        "tick": session.tick_name(),
        "video": video_json(session.video_out()),
        "stop": stop_json(session.last_stop()),
    })
}

fn video_json(video: missingno_core::video::DisplayTechnology) -> Value {
    use missingno_core::video::DisplayTechnology;
    match video {
        DisplayTechnology::Lcd {
            native,
            panel,
            pixel_aspect,
        } => json!({
            "technology": "lcd",
            "panel": panel.description(),
            "native": [native.0, native.1],
            "pixel_aspect": pixel_aspect,
        }),
        DisplayTechnology::Crt {
            standard,
            pixel_aspect,
        } => json!({
            "technology": "crt",
            "standard": standard.name(),
            "pixel_aspect": pixel_aspect,
        }),
    }
}

/// Run one stepping call and report where it stopped.
fn step(request: Request, session: &mut Session, run: fn(&mut Session) -> StopReason) {
    let stop = run(session);
    respond_step(request, session, &stop);
}

fn respond_step(request: Request, session: &Session, stop: &StopReason) {
    respond_json(
        request,
        json!({
            "pc": format!("{:x}", session.pc()),
            "frame": session.frame(),
            "stop": stop_json(stop),
        }),
    );
}

fn stop_json(stop: &StopReason) -> Value {
    match stop {
        StopReason::Completed => json!({ "reason": "completed" }),
        StopReason::Breakpoint => json!({ "reason": "breakpoint" }),
        StopReason::BudgetExhausted => json!({ "reason": "budget-exhausted" }),
        StopReason::Watch(watch) => json!({ "reason": "watch", "watch": watch_json(watch) }),
    }
}

fn watch_json(watch: &Watch) -> Value {
    let terms: Vec<Value> = watch
        .terms
        .iter()
        .map(|term| {
            let mut object = serde_json::Map::new();
            object.insert("key".into(), json!(term.key));
            if let Some(address) = term.address {
                object.insert("address".into(), json!(format!("{address:x}")));
            }
            if let Some(value) = term.value {
                object.insert("value".into(), json!(value));
            }
            Value::Object(object)
        })
        .collect();
    json!({ "terms": terms })
}

fn registers_json(session: &Session) -> Value {
    let groups: Vec<Value> = session
        .register_groups()
        .iter()
        .map(|group| {
            let registers: Vec<Value> = group.registers.iter().map(render_register).collect();
            json!({ "name": group.name, "registers": registers })
        })
        .collect();
    json!({ "groups": groups })
}

fn render_register(register: &Register) -> Value {
    let rendered = match register.style {
        ValueStyle::Hex => {
            let width = ((register.bits as usize).div_ceil(4)).max(1);
            json!(format!("{:0width$x}", register.value, width = width))
        }
        ValueStyle::Dec => json!(register.value),
        ValueStyle::Bool => json!(register.value != 0),
        ValueStyle::Flags(names) => {
            let flags: serde_json::Map<String, Value> = names
                .iter()
                .map(|flag| {
                    (
                        flag.name.to_string(),
                        json!((register.value >> flag.bit) & 1 != 0),
                    )
                })
                .collect();
            Value::Object(flags)
        }
    };
    json!({
        "name": register.name,
        "value": rendered,
        "raw": register.value,
        "bits": register.bits,
    })
}

fn regions_json(session: &Session) -> Value {
    let regions: Vec<Value> = session
        .memory_regions()
        .iter()
        .map(|region| {
            json!({
                "name": region.name,
                "start": format!("{:x}", region.start),
                "len": region.len,
            })
        })
        .collect();
    json!({ "regions": regions })
}

fn breakpoints_json(session: &Session) -> Value {
    let addresses: Vec<String> = session
        .breakpoints()
        .iter()
        .map(|address| format!("{address:x}"))
        .collect();
    json!({ "breakpoints": addresses })
}

fn watchables_json(session: &Session) -> Value {
    use missingno_core::inspect::WatchParam;
    let entries: Vec<Value> = session
        .watchables()
        .iter()
        .map(|watchable| {
            let param = match watchable.param {
                WatchParam::None => json!({ "kind": "none" }),
                WatchParam::Address => json!({ "kind": "address" }),
                WatchParam::Value { bits } => json!({ "kind": "value", "bits": bits }),
                WatchParam::AddressValue => json!({ "kind": "address-value" }),
            };
            json!({ "key": watchable.key, "label": watchable.label, "param": param })
        })
        .collect();
    json!({ "watchables": entries })
}

fn watches_json(session: &Session) -> Value {
    let watches: Vec<Value> = session.watches().iter().map(watch_json).collect();
    json!({ "watches": watches })
}

fn symbols_json(session: &Session) -> Value {
    // The seam's symbol table exposes user-created labels for iteration; the
    // generated body resolves by address, not by enumeration.
    let symbols = session.symbols();
    let entries: Vec<Value> = symbols
        .user_symbols()
        .iter()
        .map(|symbol| {
            json!({
                "bank": symbol.bank,
                "address": format!("{:04x}", symbol.address),
                "name": symbol.name,
            })
        })
        .collect();
    json!({ "symbols": entries })
}

// --- sidebar sections ---------------------------------------------------------

fn sections_json(session: &Session) -> Value {
    let sections: Vec<Value> = session
        .sidebar_sections()
        .iter()
        .map(section_json)
        .collect();
    json!({ "sections": sections })
}

fn section_json(section: &Section) -> Value {
    let blocks: Vec<Value> = section.blocks.iter().map(block_json).collect();
    json!({
        "name": section.name,
        "summary": section.summary,
        "active": section.active,
        "detail": section.detail.as_ref().map(|detail| json!({
            "text": detail.text,
            "tone": tone_name(detail.tone),
        })),
        "blocks": blocks,
    })
}

fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "neutral",
        Tone::Idle => "idle",
        Tone::Active => "active",
        Tone::Scanning => "scanning",
        Tone::Rendering => "rendering",
        Tone::Pending => "pending",
    }
}

fn block_json(block: &SectionBlock) -> Value {
    match block {
        SectionBlock::Registers(group) => json!({
            "kind": "registers",
            "name": group.name,
            "registers": group.registers.iter().map(render_register).collect::<Vec<_>>(),
        }),
        SectionBlock::Pairs(pairs) => json!({
            "kind": "pairs",
            "pairs": pairs.iter().map(pair_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Pointers(pointers) => json!({
            "kind": "pointers",
            "pointers": pointers.iter().map(pointer_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Table(table) => json!({ "kind": "table", "table": table_json(table) }),
        SectionBlock::Relations(matrix) => {
            json!({ "kind": "relations", "relations": relations_json(matrix) })
        }
        SectionBlock::Rows(rows) => json!({
            "kind": "rows",
            "rows": rows.iter().map(row_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Sweeps(sweeps) => json!({
            "kind": "sweeps",
            "sweeps": sweeps.iter().map(sweep_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Swatches(swatches) => json!({
            "kind": "swatches",
            "swatches": swatches.iter().map(swatch_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Pixels(strips) => json!({
            "kind": "pixels",
            "strips": strips.iter().map(pixel_strip_json).collect::<Vec<_>>(),
        }),
        SectionBlock::Rule => json!({ "kind": "rule" }),
    }
}

fn pair_json(pair: &RegisterPair) -> Value {
    json!({
        "high": render_register(&pair.high),
        "low": render_register(&pair.low),
        "combined": pair.combined(),
    })
}

fn pointer_json(pointer: &Pointer) -> Value {
    let mut object = render_register(&pointer.register);
    object["active"] = json!(pointer.active);
    object
}

fn table_json(table: &BitTable) -> Value {
    let columns: Vec<Value> = table
        .columns
        .iter()
        .map(|column| json!({ "name": column.name }))
        .collect();
    let rows: Vec<Value> = table
        .rows
        .iter()
        .map(|row| {
            json!({
                "name": row.name,
                "bits": row.bits,
                "tone": tone_name(row.tone),
            })
        })
        .collect();
    json!({
        "columns": columns,
        "corner": table.corner.map(|flag| json!({ "name": flag.name, "active": flag.active })),
        "rows": rows,
    })
}

fn relations_json(matrix: &PairMatrix) -> Value {
    let n = matrix.entities.len();
    let mut pairs = Vec::new();
    for j in 1..n {
        for i in 0..j {
            pairs.push(json!({
                "a": matrix.entities[i],
                "b": matrix.entities[j],
                "set": matrix.cell(i, j).set,
            }));
        }
    }
    json!({ "entities": matrix.entities, "pairs": pairs })
}

fn row_json(row: &Row) -> Value {
    json!({
        "label": row.label,
        "value": row.value,
        "active": row.active,
    })
}

fn sweep_json(sweep: &Sweep) -> Value {
    let zones: Vec<Value> = sweep
        .zones
        .iter()
        .map(|zone| json!({ "name": zone.name, "end": zone.end, "tone": tone_name(zone.tone) }))
        .collect();
    json!({
        "label": sweep.label,
        "value": sweep.value,
        "end": sweep.end,
        "zone": sweep.zone_at(sweep.value).map(|zone| zone.name),
        "zones": zones,
    })
}

fn swatch_json(swatch: &SwatchRow) -> Value {
    match swatch {
        SwatchRow::Shades { label, packed } => json!({
            "label": label,
            "kind": "shades",
            "packed": packed,
        }),
        SwatchRow::Colors { label, colors } => json!({
            "label": label,
            "kind": "colors",
            "colors": colors
                .iter()
                .map(|swatch| {
                    json!({
                        "rgb": rgb_hex(&swatch.color),
                        "raw": swatch.raw,
                    })
                })
                .collect::<Vec<_>>(),
        }),
    }
}

fn pixel_strip_json(strip: &PixelStrip) -> Value {
    match strip {
        PixelStrip::Shades { label, cells, .. } => json!({
            "label": label,
            "kind": "shades",
            "cells": cells,
        }),
        PixelStrip::Colors { label, cells, .. } => json!({
            "label": label,
            "kind": "colors",
            "cells": cells.iter().map(|cell| cell.map(|c| rgb_hex(&c))).collect::<Vec<_>>(),
        }),
        PixelStrip::Bits { label, cells, .. } => json!({
            "label": label,
            "kind": "bits",
            "cells": cells,
        }),
    }
}

fn rgb_hex(color: &rgb::RGB8) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

// --- sub-instruction stepping -------------------------------------------------

fn step_tick(request: Request, session: &mut Session, query: &str) {
    let count = match query_param(query, "count") {
        Some(text) => match text.parse::<usize>() {
            Ok(count) => count.clamp(1, MAX_TICK_COUNT),
            Err(_) => return respond_error(request, 400, "invalid count"),
        },
        None => 1,
    };
    match step_tick_json(session, count) {
        Some(body) => respond_json(request, body),
        // A core whose finest step is a whole instruction advertises no tick, so
        // the route reports 404 — the transport's "this endpoint is unsupported"
        // signal, distinct from a 400 on a malformed request.
        None => respond_error(request, 404, "this core has no sub-instruction stepping"),
    }
}

/// Advance `count` sub-instruction ticks and report the result, or `None` when
/// the core exposes no sub-instruction tick (the 404 case).
fn step_tick_json(session: &mut Session, count: usize) -> Option<Value> {
    let tick = session.tick_name()?;
    for _ in 0..count {
        session.step_tick();
    }
    let status = session.running_status();
    Some(json!({
        "pc": format!("{:x}", session.pc()),
        "tick": tick,
        "ran": count,
        "video": { "label": status.video_label, "summary": status.video_summary },
    }))
}

// --- waveforms ----------------------------------------------------------------

/// Auto-enables capture when it was off, matching the MCP `get_waveforms` tool:
/// the first read turns capture on, so the window fills from the next frame.
fn waveforms_json(session: &mut Session) -> Value {
    if session.channel_waves().is_none() {
        session.set_wave_capture(true);
    }
    match session.channel_waves() {
        Some(waves) => json!({
            "waveforms": waves.iter().map(wave_json).collect::<Vec<_>>(),
        }),
        None => json!({ "waveforms": Value::Null }),
    }
}

fn wave_json(wave: &ChannelWave) -> Value {
    json!({
        "label": wave.label,
        "rate": wave.rate,
        "depth_bits": wave.depth_bits,
        "active": wave.active,
        "levels": wave.levels,
    })
}

// --- graphics -----------------------------------------------------------------

/// Auto-enables capture when it was off, like `/waveforms`.
fn graphics_json(session: &mut Session) -> Value {
    // Enabling capture is a no-op when already on, so one decode suffices.
    session.set_graphics_capture(true);
    match session.graphics() {
        Some(graphics) => json!({ "graphics": graphics_view_json(&graphics) }),
        None => json!({ "graphics": Value::Null }),
    }
}

fn graphics_view_json(graphics: &GraphicsView) -> Value {
    json!({
        "atlases": graphics.atlases.iter().map(atlas_json).collect::<Vec<_>>(),
        "maps": graphics.maps.iter().map(map_json).collect::<Vec<_>>(),
        "objects": graphics.objects.as_ref().map(object_table_json),
    })
}

fn atlas_json(atlas: &TileAtlas) -> Value {
    let regions: Vec<Value> = atlas
        .regions
        .iter()
        .map(|region| {
            json!({
                "label": region.label,
                "start": region.start,
                "len": region.len,
                "help": region.help,
            })
        })
        .collect();
    json!({
        "label": atlas.label,
        "tile_width": atlas.tile_width,
        "tile_height": atlas.tile_height,
        "depth_bits": atlas.depth_bits,
        "palettes": palette_set_json(&atlas.palettes),
        "regions": regions,
        "tiles": (0..atlas.tile_count()).map(|tile| json!(atlas.tile_indices(tile))).collect::<Vec<_>>(),
    })
}

fn palette_set_json(set: &PaletteSet) -> Value {
    match set {
        PaletteSet::FrontendShades => json!({ "kind": "frontend-shades" }),
        PaletteSet::Owned(palettes) => json!({
            "kind": "owned",
            "palettes": palettes.iter().map(named_palette_json).collect::<Vec<_>>(),
        }),
    }
}

fn named_palette_json(palette: &NamedPalette) -> Value {
    json!({
        "label": palette.label,
        "colors": palette.colors.iter().map(rgb_hex).collect::<Vec<_>>(),
    })
}

fn map_json(map: &TileMap) -> Value {
    json!({
        "label": map.label,
        "columns": map.columns,
        "rows": map.rows,
        "atlas": map.atlas,
        "entries": map.entries.iter().map(map_entry_json).collect::<Vec<_>>(),
        "viewports": map.viewports.iter().map(viewport_json).collect::<Vec<_>>(),
    })
}

fn map_entry_json(entry: &MapEntry) -> Value {
    json!({
        "tile": entry.tile,
        "palette": entry.palette,
        "atlas": entry.atlas,
        "flip_x": entry.flip_x,
        "flip_y": entry.flip_y,
        "priority": entry.priority,
    })
}

fn viewport_json(viewport: &Viewport) -> Value {
    json!({
        "label": viewport.label,
        "x": viewport.x,
        "y": viewport.y,
        "width": viewport.width,
        "height": viewport.height,
        "wraps": viewport.wraps,
        "tone": tone_name(viewport.tone),
    })
}

fn object_table_json(table: &ObjectTable) -> Value {
    json!({
        "label": table.label,
        "atlas": table.atlas,
        "object_height": table.object_height,
        "objects": table.objects.iter().map(object_json).collect::<Vec<_>>(),
    })
}

fn object_json(object: &Object) -> Value {
    json!({
        "index": object.index,
        "x": object.x,
        "y": object.y,
        "tile": object.tile,
        "on_screen": object.on_screen,
        "palette": object.palette,
        "bank": object.bank,
        "flip_x": object.flip_x,
        "flip_y": object.flip_y,
        "priority": object.priority,
    })
}

/// Which capture gate a `/…/capture` request toggles.
enum Capture {
    Waves,
    Graphics,
}

fn capture_edit(mut request: Request, session: &mut Session, capture: Capture) {
    let on = match read_json_body(&mut request).and_then(|body| {
        body.get("on")
            .and_then(Value::as_bool)
            .ok_or_else(|| "expected { \"on\": bool }".to_string())
    }) {
        Ok(on) => on,
        Err(message) => return respond_error(request, 400, &message),
    };
    match capture {
        Capture::Waves => session.set_wave_capture(on),
        Capture::Graphics => session.set_graphics_capture(on),
    }
    respond_json(request, json!({ "capture": on }));
}

/// Read a `{ "path": "..." }` body, returning the path it names.
fn read_path_body(request: &mut Request) -> Result<String, String> {
    let body = read_json_body(request)?;
    body.get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "expected { \"path\": string }".to_string())
}

fn recording_replay(mut request: Request, session: &mut Session) {
    let path = match read_path_body(&mut request) {
        Ok(path) => path,
        Err(message) => return respond_error(request, 400, &message),
    };
    match session.replay_recording(std::path::Path::new(&path)) {
        Ok(frames) => respond_json(request, json!({ "replayed": path, "frames": frames })),
        Err(message) => respond_error(request, 400, &message),
    }
}

fn disassembly(request: Request, session: &Session, query: &str) {
    let at = match query_param(query, "at") {
        Some(text) => match parse_hex(&text) {
            Ok(address) => address,
            Err(message) => return respond_error(request, 400, &message),
        },
        None => session.pc(),
    };
    let count = match query_param(query, "count") {
        Some(text) => match text.parse::<usize>() {
            Ok(count) => count.clamp(1, MAX_DISASM_COUNT),
            Err(_) => return respond_error(request, 400, "invalid count"),
        },
        None => DEFAULT_DISASM_COUNT,
    };
    match session.disassembly(at, count) {
        Ok(lines) => respond_json(
            request,
            json!({
                "at": format!("{at:x}"),
                "lines": lines.iter().map(disasm_json).collect::<Vec<_>>(),
            }),
        ),
        Err(message) => respond_error(request, 400, &message),
    }
}

fn disasm_json(line: &DisasmLine) -> Value {
    let hex: Vec<String> = line
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    json!({
        "address": format!("{:x}", line.address),
        "kind": if line.is_data { "data" } else { "instruction" },
        "text": line.text,
        "bytes": hex,
        "length": line.length,
    })
}

/// The current frame in its pre-resolution domain: DMG shade indices (0-3), CGB
/// RGB555 words as hex, or palette indices. `null` when the core exposes none.
fn frame_raw_json(session: &Session) -> Value {
    use missingno_core::video::RawFrame;
    match session.frame_raw() {
        Some(RawFrame::Shade2 {
            width,
            height,
            pixels,
        }) => json!({ "format": "shade2", "width": width, "height": height, "pixels": pixels }),
        Some(RawFrame::Palette {
            width,
            height,
            pixels,
        }) => json!({ "format": "palette", "width": width, "height": height, "pixels": pixels }),
        Some(RawFrame::Rgb555 {
            width,
            height,
            pixels,
        }) => {
            let hex: Vec<String> = pixels.iter().map(|word| format!("{word:04x}")).collect();
            json!({ "format": "rgb555", "width": width, "height": height, "pixels": hex })
        }
        None => json!({ "format": Value::Null }),
    }
}

fn frame_bitmap(request: Request, session: &Session) {
    let frame = session.frame_rgba();
    let response = Response::from_data(frame.pixels.to_vec())
        .with_header(header("Content-Type: application/octet-stream"))
        .with_header(header(&format!("X-Frame-Width: {}", frame.width)))
        .with_header(header(&format!("X-Frame-Height: {}", frame.height)));
    let _ = request.respond(response);
}

fn memory(request: Request, session: &Session, method: &Method, path: &str) {
    if *method != Method::Get {
        return respond_error(request, 405, "method not allowed");
    }
    let rest = &path["/memory/".len()..];
    let (address_text, length_text) = match rest.split_once('/') {
        Some((address, length)) => (address, Some(length)),
        None => (rest, None),
    };
    let address = match parse_hex(address_text) {
        Ok(address) => address,
        Err(message) => return respond_error(request, 400, &message),
    };
    let length = match length_text {
        Some(text) => match text.parse::<u32>() {
            Ok(length) if (1..=MAX_MEMORY_LEN).contains(&length) => length,
            _ => return respond_error(request, 400, "invalid length (1-4096)"),
        },
        None => 1,
    };
    let bytes = session.memory(address, length);
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    respond_json(
        request,
        json!({
            "address": format!("{address:x}"),
            "length": length,
            "bytes": bytes,
            "hex": hex,
        }),
    );
}

fn breakpoint_edit(request: Request, session: &mut Session, method: &Method, path: &str) {
    let address = match parse_hex(&path["/breakpoints/".len()..]) {
        Ok(address) => address,
        Err(message) => return respond_error(request, 400, &message),
    };
    match *method {
        Method::Put => match session.set_breakpoint(address) {
            Ok(()) => respond_json(request, json!({ "set": format!("{address:x}") })),
            Err(message) => respond_error(request, 400, &message),
        },
        Method::Delete => {
            session.clear_breakpoint(address);
            respond_json(request, json!({ "cleared": format!("{address:x}") }));
        }
        _ => respond_error(request, 405, "method not allowed"),
    }
}

/// The two edits `/watches` accepts, one per method it answers to.
enum WatchEdit {
    Add,
    Remove,
}

fn watch_edit(mut request: Request, session: &mut Session, edit: WatchEdit) {
    let terms = match read_json_body(&mut request).and_then(|body| parse_watch_terms(&body)) {
        Ok(terms) => terms,
        Err(message) => return respond_error(request, 400, &message),
    };
    let (result, field) = match edit {
        WatchEdit::Add => (session.add_watch(terms), "added"),
        WatchEdit::Remove => (session.remove_watch(terms), "removed"),
    };
    respond_result(
        request,
        result.map(|watch| json!({ field: watch_json(&watch) })),
    );
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn respond_json(request: Request, body: Value) {
    let json = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string());
    let response =
        Response::from_string(json).with_header(header("Content-Type: application/json"));
    let _ = request.respond(response);
}

fn respond_error(request: Request, code: u16, message: &str) {
    let json = serde_json::to_string(&json!({ "error": message })).unwrap();
    let response = Response::from_string(json)
        .with_status_code(StatusCode(code))
        .with_header(header("Content-Type: application/json"));
    let _ = request.respond(response);
}

fn header(text: &str) -> Header {
    text.parse::<Header>().expect("valid header literal")
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    #[test]
    fn a_query_string_never_changes_the_routed_path() {
        assert_eq!(split_url("/run"), ("/run", ""));
        assert_eq!(split_url("/run?x=1"), ("/run", "x=1"));
        assert_eq!(split_url("/run?"), ("/run", ""));
        assert_eq!(
            split_url("/disassembly?at=100&count=4"),
            ("/disassembly", "at=100&count=4")
        );
        assert_eq!(split_url("/memory/c000/16"), ("/memory/c000/16", ""));
    }

    #[test]
    fn both_routing_passes_split_the_same_way() {
        // `session_route` and `handle` share one splitter, so the route a
        // request lands on is the same with or without a query string.
        for url in ["/run", "/pause", "/state/save", "/status", "/step"] {
            assert_eq!(split_url(url).0, split_url(&format!("{url}?x=1")).0);
        }
    }
}

/// The JSON these endpoints emit is exercised over a real Game Boy session
/// built through the factory — the same path the server drives — so the shapes
/// stay pinned without standing up a socket.
#[cfg(all(test, feature = "gb"))]
mod tests {
    use super::*;
    use std::path::Path;

    use missingno_session::Session;

    /// A 32 KiB all-NOP `.gb` ROM: the extension makes the registry claim it,
    /// and the DMG core boots to PC 0x0100.
    fn gb_session() -> Session {
        let rom = vec![0x00u8; 0x8000];
        let console = missingno_session::factory::create_console(Path::new("test.gb"), &rom)
            .expect("gb factory claims a .gb ROM");
        Session::new(console.into_debugger())
    }

    #[test]
    fn status_reports_the_tick_name() {
        let session = gb_session();
        let status = status_json(&session);
        assert_eq!(status["tick"], json!("dot"));
    }

    #[test]
    fn sections_carry_named_sections_with_typed_blocks() {
        let session = gb_session();
        let value = sections_json(&session);
        let sections = value["sections"].as_array().expect("a sections array");
        assert!(!sections.is_empty());
        // Every section names itself and carries typed blocks (each tagged).
        for section in sections {
            assert!(section["name"].is_string());
            for block in section["blocks"].as_array().expect("blocks array") {
                assert!(block["kind"].is_string(), "every block carries a kind tag");
            }
        }
    }

    #[test]
    fn step_tick_advances_and_names_the_tick() {
        let mut session = gb_session();
        let pc0 = session.pc();
        // Four dots complete exactly one NOP, advancing the PC by one.
        let body = step_tick_json(&mut session, 4).expect("gb names a tick");
        assert_eq!(body["tick"], json!("dot"));
        assert_eq!(body["ran"], json!(4));
        assert_eq!(body["pc"], json!(format!("{:x}", pc0 + 1)));
    }

    /// A CGB session: the CGB header flag makes the factory boot the colour core.
    fn cgb_session() -> Session {
        let mut rom = vec![0x00u8; 0x8000];
        rom[0x143] = 0xC0;
        let console = missingno_session::factory::create_console(Path::new("test.gbc"), &rom)
            .expect("gb factory claims a .gbc ROM");
        Session::new(console.into_debugger())
    }

    #[test]
    fn frame_raw_dmg_serves_shade_indices() {
        let session = gb_session();
        let raw = frame_raw_json(&session);
        assert_eq!(raw["format"], json!("shade2"));
        assert_eq!(raw["width"], json!(160));
        assert_eq!(raw["height"], json!(144));
        let pixels = raw["pixels"].as_array().expect("pixels array");
        assert_eq!(pixels.len(), 160 * 144);
        // Every DMG pixel is a 2-bit shade (0-3).
        assert!(pixels.iter().all(|p| p.as_u64().is_some_and(|v| v <= 3)));
    }

    #[test]
    fn frame_raw_cgb_serves_rgb555_words() {
        let session = cgb_session();
        let raw = frame_raw_json(&session);
        assert_eq!(raw["format"], json!("rgb555"));
        assert_eq!(raw["width"], json!(160));
        let pixels = raw["pixels"].as_array().expect("pixels array");
        assert_eq!(pixels.len(), 160 * 144);
        // The boot fade seeds the screen white ($7FFF), a real RGB555 word.
        assert_eq!(pixels[0], json!("7fff"));
    }

    #[test]
    fn waveforms_auto_enable_on_first_read() {
        let mut session = gb_session();
        // The first read turns capture on, so the Game Boy's four channels appear
        // (their windows fill from the next frame — empty-until-stepped is fine).
        let value = waveforms_json(&mut session);
        let waves = value["waveforms"]
            .as_array()
            .expect("gb captures waveforms");
        assert_eq!(waves.len(), 4);
        assert!(waves[0]["label"].is_string());
        assert!(waves[0]["levels"].is_array());
    }

    #[test]
    fn graphics_auto_enable_and_decode_atlases() {
        let mut session = gb_session();
        session.step_frame();
        let value = graphics_json(&mut session);
        let graphics = &value["graphics"];
        assert!(graphics.is_object(), "gb exposes graphics surfaces");
        let atlases = graphics["atlases"].as_array().expect("atlases array");
        assert!(!atlases.is_empty());
        // A tile is a flat index array; a Game Boy tile is 8×8 = 64 indices.
        let tiles = atlases[0]["tiles"].as_array().expect("tiles array");
        assert_eq!(tiles[0].as_array().expect("index array").len(), 64);
    }
}
