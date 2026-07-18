//! The HTTP transport over a [`Session`]. This module only routes requests and
//! encodes JSON — every debugger operation is a `Session` call. The endpoints
//! are all generic: they render whatever the seam schema reports, so the same
//! routes serve any core.

use missingno_core::inspect::{Register, ValueStyle, Watch, WatchTerm};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::session::{DisasmLine, Session, StopReason};

/// Cap on a single `/memory` read, so a bad length can't allocate unbounded.
const MAX_MEMORY_LEN: u32 = 0x1000;
/// Default and cap for a `/disassembly` window.
const DEFAULT_DISASM_COUNT: usize = 16;
const MAX_DISASM_COUNT: usize = 256;

/// The outcome of offering a request to a family extension handler: the
/// handler either served it, or declined and handed the request back for the
/// next handler (and finally the generic routes) to try.
pub enum Dispatch {
    Handled,
    Declined(Request),
}

/// A family-specific route handler mounted ahead of the generic routes. It
/// owns the routes its core exposes (register/memory shapes a generic client
/// cannot express) and declines everything else. `path` is the request path
/// with any query string already split off.
pub type Extension = fn(&mut Session, Request, &Method, &str) -> Dispatch;

/// Serve `session` on `127.0.0.1:<port>` until the process is killed. Each
/// request is offered to `extensions` in order before the generic routes.
pub fn serve(mut session: Session, port: u16, extensions: Vec<Extension>) -> std::io::Result<()> {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address)
        .map_err(|e| std::io::Error::other(format!("failed to bind {address}: {e}")))?;
    eprintln!("headless debugger ready: {}", session.game_title());
    eprintln!("listening on http://{address}");
    for request in server.incoming_requests() {
        handle(request, &mut session, &extensions);
    }
    Ok(())
}

fn handle(request: Request, session: &mut Session, extensions: &[Extension]) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (url, String::new()),
    };

    let mut request = request;
    for extension in extensions {
        match extension(session, request, &method, &path) {
            Dispatch::Handled => return,
            Dispatch::Declined(returned) => request = returned,
        }
    }

    match (&method, path.as_str()) {
        (Method::Get, "/status") => respond_json(request, status_json(session)),
        (Method::Get, "/registers") => respond_json(request, registers_json(session)),
        (Method::Get, "/regions") => respond_json(request, regions_json(session)),
        (Method::Get, "/breakpoints") => respond_json(request, breakpoints_json(session)),
        (Method::Get, "/watchables") => respond_json(request, watchables_json(session)),
        (Method::Get, "/watches") => respond_json(request, watches_json(session)),
        (Method::Get, "/symbols") => respond_json(request, symbols_json(session)),
        (Method::Get, "/disassembly") => disassembly(request, session, &query),
        (Method::Get, "/frame/bitmap") => frame_bitmap(request, session),

        (Method::Post, "/step") => {
            let stop = session.step();
            respond_step(request, session, &stop);
        }
        (Method::Post, "/step-over") => {
            let stop = session.step_over();
            respond_step(request, session, &stop);
        }
        (Method::Post, "/step-frame") => {
            let stop = session.step_frame();
            respond_step(request, session, &stop);
        }
        (Method::Post, "/reset") => {
            session.reset();
            respond_json(request, status_json(session));
        }

        (Method::Put, "/watches") | (Method::Delete, "/watches") => {
            watch_edit(request, session, method)
        }

        _ if path.starts_with("/memory/") => memory(request, session, &method, &path),
        _ if path.starts_with("/breakpoints/") => breakpoint_edit(request, session, &method, &path),

        _ => respond_error(request, 404, "not found"),
    }
}

fn status_json(session: &Session) -> Value {
    json!({
        "pc": format!("{:x}", session.pc()),
        "frame": session.frame(),
        "title": session.game_title(),
        "stop": stop_json(session.last_stop()),
    })
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

fn disassembly(request: Request, session: &Session, query: &str) {
    let at = match query_param(query, "at") {
        Some(text) => match parse_hex_u32(&text) {
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
    let address = match parse_hex_u32(address_text) {
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
    let address = match parse_hex_u32(&path["/breakpoints/".len()..]) {
        Ok(address) => address,
        Err(message) => return respond_error(request, 400, &message),
    };
    match *method {
        Method::Put => {
            session.set_breakpoint(address);
            respond_json(request, json!({ "set": format!("{address:x}") }));
        }
        Method::Delete => {
            session.clear_breakpoint(address);
            respond_json(request, json!({ "cleared": format!("{address:x}") }));
        }
        _ => respond_error(request, 405, "method not allowed"),
    }
}

fn watch_edit(mut request: Request, session: &mut Session, method: Method) {
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return respond_error(request, 400, "could not read request body");
    }
    let terms = match parse_watch_terms(&body) {
        Ok(terms) => terms,
        Err(message) => return respond_error(request, 400, &message),
    };
    let result = match method {
        Method::Put => session.add_watch(terms),
        Method::Delete => session.remove_watch(terms),
        _ => return respond_error(request, 405, "method not allowed"),
    };
    match result {
        Ok(watch) => {
            let field = if method == Method::Put {
                "added"
            } else {
                "removed"
            };
            respond_json(request, json!({ field: watch_json(&watch) }));
        }
        Err(message) => respond_error(request, 400, &message),
    }
}

fn parse_watch_terms(body: &str) -> Result<Vec<WatchTerm>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    if let Some(terms) = value.get("terms").and_then(|terms| terms.as_array()) {
        terms.iter().map(parse_watch_term).collect()
    } else {
        Ok(vec![parse_watch_term(&value)?])
    }
}

fn parse_watch_term(value: &Value) -> Result<WatchTerm, String> {
    let key = value
        .get("key")
        .and_then(|key| key.as_str())
        .ok_or("watch term missing 'key'")?
        .to_string();
    Ok(WatchTerm {
        key,
        address: parse_optional_u32(value.get("address"))?,
        value: parse_optional_u32(value.get("value"))?,
    })
}

fn parse_optional_u32(value: Option<&Value>) -> Result<Option<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| "number out of range".to_string()),
        Some(Value::String(text)) => parse_hex_u32(text).map(Some),
        Some(_) => Err("expected a number or hex string".to_string()),
    }
}

fn parse_hex_u32(text: &str) -> Result<u32, String> {
    let trimmed = text
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u32::from_str_radix(trimmed, 16).map_err(|_| format!("invalid hex value: {text}"))
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
