//! An MCP-over-stdio transport that drives a running missingno app window. It is
//! a hand-rolled minimal JSON-RPC 2.0 loop — newline-delimited messages in on
//! stdin, responses out on stdout — implementing the MCP server basics
//! (`initialize`, `tools/list`, `tools/call`, graceful shutdown) against
//! protocol version "2024-11-05".
//!
//! stdout is the protocol channel: every log line goes to stderr. The frontend
//! twin of the debugger's idle server: it starts attached to nothing, advertising
//! only the tools that reach a window — `attach`/`detach`/`status` — and once
//! `attach` reaches an app window it mirrors that window's own tool surface,
//! forwarding `tools/list` and `tools/call` frames verbatim over the socket.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The MCP protocol version whose message shapes this server targets.
const PROTOCOL_VERSION: &str = "2024-11-05";

const NO_WINDOWS: &str = "no running app windows are reachable (a window publishes its automation \
                          surface only when launched with --allow-ui-automation or the equivalent \
                          setting is on)";

/// What this server is currently driving: nothing, or one attached app window.
struct State {
    #[cfg(unix)]
    client: Option<crate::ui_socket::UiClient>,
}

impl State {
    fn new() -> Self {
        State {
            #[cfg(unix)]
            client: None,
        }
    }

    fn is_attached(&self) -> bool {
        #[cfg(unix)]
        return self.client.is_some();
        #[cfg(not(unix))]
        false
    }

    fn description(&self) -> String {
        #[cfg(unix)]
        if let Some(client) = &self.client {
            return client.info().summary();
        }
        "idle".to_string()
    }
}

/// Serve the automation MCP server over stdio until stdin reaches EOF or a
/// `shutdown` request arrives.
pub fn serve() -> io::Result<()> {
    let mut state = State::new();
    eprintln!("mcp: idle (not attached) ready on stdio");
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let was_attached = state.is_attached();
        let (response, exit) = handle_message(&line, &mut state);
        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response).unwrap())?;
            stdout.flush()?;
        }
        // Attaching/detaching swaps the whole tool set: tell the client to
        // re-list, or it keeps showing the idle tools.
        if state.is_attached() != was_attached {
            let notice = json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
            writeln!(stdout, "{notice}")?;
            stdout.flush()?;
        }
        if exit {
            break;
        }
    }
    Ok(())
}

/// Dispatch one JSON-RPC message. Returns the response to emit (if any) and
/// whether the loop should exit afterwards.
fn handle_message(line: &str, state: &mut State) -> (Option<Value>, bool) {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => {
            return (
                Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {error}"),
                )),
                false,
            );
        }
    };

    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    // Notifications (no id) are accepted silently.
    let Some(id) = id else {
        return (None, false);
    };

    match method {
        "initialize" => (Some(success(id, initialize_result(state))), false),
        "ping" => (Some(success(id, json!({}))), false),
        "tools/list" => (Some(success(id, tools_list(state))), false),
        "tools/call" => (Some(success(id, tools_call(state, &params))), false),
        "shutdown" => (Some(success(id, Value::Null)), true),
        other => (
            Some(error_response(
                id,
                -32601,
                &format!("method not found: {other}"),
            )),
            false,
        ),
    }
}

fn initialize_result(state: &State) -> Value {
    let name = if state.is_attached() {
        format!("missingno-remote ({})", state.description())
    } else {
        "missingno-remote (idle)".to_string()
    };
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": {
            "name": name,
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

// --- tools/list ---------------------------------------------------------------

fn tools_list(state: &mut State) -> Value {
    #[cfg(unix)]
    if state.client.is_some() {
        // Attached: mirror the window's own surface, plus the management tools so
        // the window can be swapped without restarting the server.
        let result = state
            .client
            .as_mut()
            .expect("attached")
            .request("tools/list", json!({}));
        match result {
            Ok(list) => {
                let mut tools = list
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                tools.push(attach_tool());
                tools.push(detach_tool());
                return json!({ "tools": tools });
            }
            // The window went away mid-session: drop to idle and advertise the
            // idle set rather than reporting the loss here.
            Err(crate::ui_socket::RequestError::Transport(_)) => state.client = None,
            // Alive but declined to list — advertise the idle set this turn.
            Err(crate::ui_socket::RequestError::Answered(_)) => {}
        }
    }
    json!({ "tools": [attach_tool(), detach_tool(), status_tool()] })
}

// --- tools/call ---------------------------------------------------------------

fn tools_call(state: &mut State, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "attach" => attach(state, &args),
        "detach" => detach(state),
        _ => forward_or_idle(state, name, params),
    }
}

/// Forward a call to the attached window, or answer from the idle server when
/// nothing is attached. `status` is left to the window's own tool while attached.
#[cfg_attr(not(unix), allow(unused_variables))]
fn forward_or_idle(state: &mut State, name: &str, params: &Value) -> Value {
    #[cfg(unix)]
    if state.client.is_some() {
        let result = state
            .client
            .as_mut()
            .expect("attached")
            .request("tools/call", params.clone());
        return match result {
            Ok(value) => value,
            // The window is alive; its tool answered with an error.
            Err(crate::ui_socket::RequestError::Answered(message)) => error_result(&message),
            Err(crate::ui_socket::RequestError::Transport(error)) => {
                state.client = None;
                error_result(&format!("the app window is gone: {error}"))
            }
        };
    }
    if name == "status" {
        text_result(&idle_status_text())
    } else {
        error_result(&format!("not attached; call attach first (tool: {name})"))
    }
}

#[cfg(unix)]
fn attach(state: &mut State, args: &Value) -> Value {
    use crate::ui_socket::{UiClient, discover};

    let connected = match args.get("path").and_then(Value::as_str) {
        Some(path) => UiClient::connect(std::path::Path::new(path)).map_err(String::from),
        None => {
            let wanted = args.get("pid").and_then(Value::as_u64);
            let windows = discover();
            let chosen = match wanted {
                Some(pid) => match windows.into_iter().find(|w| u64::from(w.pid) == pid) {
                    Some(window) => window,
                    None => {
                        return error_result(&format!("no reachable app window with pid {pid}"));
                    }
                },
                None => match windows.len() {
                    0 => return error_result(NO_WINDOWS),
                    1 => windows.into_iter().next().expect("one window"),
                    // Which window to drive is the agent's choice, never a guess.
                    _ => {
                        return error_result(&format!(
                            "several app windows are reachable; give a pid:\n{}",
                            window_list(&windows)
                        ));
                    }
                },
            };
            UiClient::connect(&chosen.path).map_err(String::from)
        }
    };
    match connected {
        Ok(client) => {
            let summary = client.info().summary();
            state.client = Some(client);
            text_result(&format!("attached to {summary}"))
        }
        Err(error) => error_result(&format!("could not attach: {error}")),
    }
}

#[cfg(not(unix))]
fn attach(_state: &mut State, _args: &Value) -> Value {
    error_result("attaching to an app window needs Unix domain sockets")
}

fn detach(state: &mut State) -> Value {
    #[cfg(unix)]
    {
        match state.client.take() {
            Some(client) => text_result(&format!("detached from {}", client.info().summary())),
            None => error_result("not attached"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = state;
        error_result("not attached")
    }
}

/// The idle server's status: what it can do, and which windows it can reach now.
fn idle_status_text() -> String {
    let mut body =
        "idle: not attached. Call attach to drive a running app window in real time.".to_string();
    #[cfg(unix)]
    {
        let windows = crate::ui_socket::discover();
        body.push_str("\n\n");
        if windows.is_empty() {
            body.push_str(NO_WINDOWS);
        } else {
            body.push_str("reachable app windows:\n");
            body.push_str(&window_list(&windows));
        }
    }
    body
}

#[cfg(unix)]
fn window_list(windows: &[crate::ui_socket::UiInfo]) -> String {
    windows
        .iter()
        .map(|window| window.summary())
        .collect::<Vec<_>>()
        .join("\n")
}

// --- management tool advertisements -------------------------------------------

/// The schema of a tool that takes no arguments.
fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn attach_tool() -> Value {
    json!({
        "name": "attach",
        "description": "Attach to a running missingno app window that publishes a UI-automation \
                        socket, and drive its live window through its own tools. `status` lists \
                        reachable windows; give `pid` to pick one, or omit it when exactly one is \
                        reachable. Give `path` for a socket outside the usual runtime directory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "process id of the window to attach to" },
                "path": {
                    "type": "string",
                    "description": "socket path, when it is not in the usual runtime directory",
                },
            },
        },
    })
}

fn detach_tool() -> Value {
    json!({
        "name": "detach",
        "description": "Detach from the attached app window, returning the server to idle. The \
                        window keeps running.",
        "inputSchema": no_arguments(),
    })
}

fn status_tool() -> Value {
    json!({
        "name": "status",
        "description": "Whether the server is idle or attached, and which app windows are \
                        reachable right now — each with its pid and version.",
        "inputSchema": no_arguments(),
    })
}

// --- tool result bodies -------------------------------------------------------

fn text_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn error_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn dispatch(line: &str, state: &mut State) -> Value {
        handle_message(line, state).0.expect("a response")
    }

    #[test]
    fn initialize_names_itself_idle() {
        let mut state = State::new();
        let response = dispatch(
            r#"{ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }"#,
            &mut state,
        );
        let name = response["result"]["serverInfo"]["name"].as_str().unwrap();
        assert!(name.contains("idle"), "idle handshake, got {name}");
        assert_eq!(
            response["result"]["protocolVersion"],
            json!("2024-11-05"),
            "protocol version"
        );
    }

    #[test]
    fn idle_advertises_the_management_tools() {
        let mut state = State::new();
        let response = dispatch(
            r#"{ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }"#,
            &mut state,
        );
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["attach", "detach", "status"], "idle tools");
    }

    #[test]
    fn attach_to_a_missing_socket_is_an_error_result() {
        let mut state = State::new();
        let missing = std::env::temp_dir().join("missingno-remote-nonexistent.sock");
        let call = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "attach", "arguments": { "path": missing } },
        });
        let response = dispatch(&call.to_string(), &mut state);
        assert_eq!(response["result"]["isError"], json!(true), "{response:?}");
        assert!(!state.is_attached(), "attach failed, stays idle");
    }

    #[test]
    fn a_call_while_idle_is_rejected() {
        let mut state = State::new();
        let response = dispatch(
            r#"{ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "ui_tree", "arguments": {} } }"#,
            &mut state,
        );
        assert_eq!(response["result"]["isError"], json!(true), "{response:?}");
    }

    #[test]
    fn a_bad_frame_is_a_parse_error() {
        let mut state = State::new();
        let response = dispatch("{ not json", &mut state);
        assert_eq!(response["error"]["code"], json!(-32700), "{response:?}");
    }

    #[test]
    fn an_unknown_method_is_reported() {
        let mut state = State::new();
        let response = dispatch(
            r#"{ "jsonrpc": "2.0", "id": 5, "method": "nope" }"#,
            &mut state,
        );
        assert_eq!(response["error"]["code"], json!(-32601), "{response:?}");
    }

    #[test]
    fn a_notification_draws_no_response() {
        let mut state = State::new();
        let outcome = handle_message(
            r#"{ "jsonrpc": "2.0", "method": "notifications/initialized" }"#,
            &mut state,
        );
        assert!(outcome.0.is_none(), "notifications are silent");
    }
}
