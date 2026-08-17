//! The stdio JSON-RPC transport scaffolding MCP servers share: a hand-rolled
//! minimal JSON-RPC 2.0 loop — newline-delimited messages in on stdin, responses
//! out on stdout — implementing the MCP server basics (`initialize`, `ping`,
//! `tools/list`, `tools/call`, graceful shutdown) against protocol version
//! "2024-11-05".
//!
//! stdout is the protocol channel: a server logs to stderr. Only the envelope
//! lives here — what a server advertises, how it answers a call, and what its
//! tool set depends on are the server's own, supplied through [`Server`].

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The MCP protocol version whose message shapes this transport targets.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// What a stdio MCP server supplies to the transport: how it names itself in the
/// handshake, what it advertises, and how it answers a call.
pub trait Server {
    /// The version the server reports in the handshake.
    const VERSION: &'static str;

    /// Whatever the advertised tool set depends on. The transport reads it
    /// either side of a message and tells the client to re-list when it changes,
    /// so a server that swaps what it drives never leaves a stale tool set up.
    type ToolSetId: PartialEq;

    /// What the server calls itself in the handshake.
    fn name(&self) -> String;

    fn tool_set_id(&self) -> Self::ToolSetId;

    /// The `tools/list` result body.
    fn list_tools(&mut self) -> Value;

    /// The `tools/call` result body, over the request's `params`.
    fn call_tool(&mut self, params: &Value) -> Value;
}

/// Serve `server` over stdio until stdin reaches EOF or a `shutdown` request
/// arrives.
pub fn serve<S: Server>(server: &mut S) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let tool_set_before = server.tool_set_id();
        let (response, exit) = handle_message(&line, server);
        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response).unwrap())?;
            stdout.flush()?;
        }
        if server.tool_set_id() != tool_set_before {
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
pub fn handle_message<S: Server>(line: &str, server: &mut S) -> (Option<Value>, bool) {
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

    // Notifications (no id) are accepted silently; the only one we expect is
    // `notifications/initialized`.
    let Some(id) = id else {
        return (None, false);
    };

    match method {
        "initialize" => (
            Some(success(id, initialize_result(&server.name(), S::VERSION))),
            false,
        ),
        "ping" => (Some(success(id, json!({}))), false),
        "tools/list" => (Some(success(id, server.list_tools())), false),
        "tools/call" => (Some(success(id, server.call_tool(&params))), false),
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

/// The handshake result: the protocol version, the tool capability, and what the
/// server calls itself.
pub fn initialize_result(name: &str, version: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": {
            "name": name,
            "version": version,
        },
    })
}

pub fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The schema of a tool that takes no arguments.
pub fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}
