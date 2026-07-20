//! An MCP-over-stdio transport over a session's agent tool surface, for an agent
//! to drive the headless debugger as a tool server. It is a hand-rolled minimal
//! JSON-RPC 2.0 loop — newline-delimited messages in on stdin, responses out on
//! stdout — implementing the MCP server basics (`initialize`, `tools/list`,
//! `tools/call`, graceful shutdown) against protocol version "2024-11-05".
//!
//! stdout is the protocol channel: every log line goes to stderr. The tools
//! themselves belong to the session, not to this transport: the server adds only
//! the management tools that pick what it drives — a ROM it loads, or a session
//! another process published.

use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Value, json};

use missingno_session::factory::{self, LoadOptions};
use missingno_session::shared::SharedSession;
use missingno_session::tools::{
    Tool, ToolOutcome, call_session_tool, outcome_json, session_tools, text,
};

/// What the server is currently driving. This transport is a client either way:
/// a locally hosted session is reached through its handle, an attached one
/// through the socket a host published, and the tool surface is the same.
enum Host {
    /// A console this server created and owns, and the core the factory
    /// recognised it as.
    Local {
        shared: SharedSession,
        core_name: &'static str,
    },
    /// Someone else's running session, driven over its attach socket.
    #[cfg(unix)]
    Attached { client: missingno_session::attach::AttachClient },
}

impl Host {
    fn description(&self) -> String {
        match self {
            Host::Local { core_name, .. } => core_name.to_string(),
            #[cfg(unix)]
            Host::Attached { client } => {
                format!("attached: {}", client.info().summary())
            }
        }
    }
}

/// The MCP protocol version whose message shapes this server targets.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Serve a preloaded shared `session` as an MCP tool server over stdio.
/// `core_name` names the core in `status` and the server handshake.
pub fn serve(session: SharedSession, core_name: &'static str) -> io::Result<()> {
    run(Some(Host::Local {
        shared: session,
        core_name,
    }))
}

/// Serve an idle MCP tool server: it starts with no machine, advertising only
/// `load_rom`/`attach`/`eject`/`status`, and gains the full tool set once
/// `load_rom` recognises a ROM or `attach` reaches a running session. One idle
/// server serves any ROM and any host.
pub fn serve_idle() -> io::Result<()> {
    run(None)
}

/// The stdio serve loop, over an optional loaded session, until stdin reaches
/// EOF or a `shutdown` request arrives. Every tool is a [`Session`] call: the
/// transport is Session-only by construction, with no family-specific escape
/// hatch.
fn run(mut loaded: Option<Host>) -> io::Result<()> {
    match &loaded {
        Some(host) => eprintln!("mcp: {} ready on stdio", host.description()),
        None => eprintln!("mcp: idle (no ROM loaded) ready on stdio"),
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (response, exit) = handle_message(&line, &mut loaded);
        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response).unwrap())?;
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
fn handle_message(line: &str, loaded: &mut Option<Host>) -> (Option<Value>, bool) {
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
        "initialize" => (Some(success(id, initialize_result(loaded))), false),
        "ping" => (Some(success(id, json!({}))), false),
        "tools/list" => (Some(success(id, tools_list(loaded))), false),
        "tools/call" => (Some(success(id, tools_call(loaded, &params))), false),
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

fn initialize_result(loaded: &Option<Host>) -> Value {
    let name = match loaded {
        Some(host) => format!("missingno-debugger ({})", host.description()),
        None => "missingno-debugger (idle)".to_string(),
    };
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
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

fn tools_list(loaded: &mut Option<Host>) -> Value {
    let tools: Vec<Value> = match loaded {
        // Idle: only the tools that reach a machine, plus an idle-aware status.
        None => [load_rom_tool(), attach_tool(), eject_tool(), status_tool()]
            .iter()
            .map(Tool::to_json)
            .collect(),
        // Driving something: the session's own surface, plus the management
        // tools so the machine can be swapped without restarting the server.
        Some(host) => {
            let session_tools = match host {
                Host::Local { shared, .. } => session_tools(&shared.handle())
                    .iter()
                    .map(Tool::to_json)
                    .collect(),
                // An attached host advertises its own surface; forward it rather
                // than guessing what the other process serves.
                #[cfg(unix)]
                Host::Attached { client } => client
                    .request("tools/list", json!({}))
                    .ok()
                    .and_then(|result| result.get("tools").and_then(Value::as_array).cloned())
                    .unwrap_or_default(),
            };
            session_tools
                .into_iter()
                .chain(
                    [load_rom_tool(), attach_tool(), eject_tool()]
                        .iter()
                        .map(Tool::to_json),
                )
                .collect()
        }
    };
    json!({ "tools": tools })
}

/// The idle-server tool that recognises and loads a ROM.
fn load_rom_tool() -> Tool {
    Tool {
        name: "load_rom",
        description: "Load a ROM by filesystem path and begin debugging it. The core is \
                      recognised from the file across all enabled cores. Optional `tv_standard` \
                      (ntsc/pal/secam) overrides the Atari VCS broadcast-standard auto-detection."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "filesystem path to the ROM" },
                "tv_standard": {
                    "type": "string",
                    "enum": ["ntsc", "pal", "secam"],
                    "description": "VCS broadcast-standard override",
                },
            },
            "required": ["path"],
        }),
    }
}

/// The tool that unloads the current machine and returns the server to idle.
fn eject_tool() -> Tool {
    Tool {
        name: "eject",
        description: "Unload the current ROM, or detach from an attached session, returning the \
                      server to idle."
            .into(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }
}

/// The tool that joins a session another process is already running.
fn attach_tool() -> Tool {
    Tool {
        name: "attach",
        description: "Attach to a session another process is already running (an app window that \
                      allows external debugger clients), and drive that live machine instead of \
                      loading a private copy. `status` lists reachable sessions; give `pid` to \
                      pick one, or omit it when exactly one is reachable. Whatever you do to the \
                      session, its own window shows."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pid": {
                    "type": "integer",
                    "description": "process id of the session to attach to",
                },
                "path": {
                    "type": "string",
                    "description": "socket path, when it is not in the usual runtime directory",
                },
            },
        }),
    }
}

/// The idle-aware status tool (the loaded tool set carries its own `status`).
fn status_tool() -> Tool {
    Tool {
        name: "status",
        description: "The loaded ROM's core, title, program counter, frame count, and last stop \
                      reason — or idle when no ROM is loaded."
            .into(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }
}

// --- tools/call ---------------------------------------------------------------

fn tools_call(loaded: &mut Option<Host>, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let outcome = match name {
        "load_rom" => load_rom(loaded, &args),
        "attach" => attach(loaded, &args),
        "eject" => eject(loaded),
        _ => match loaded {
            Some(Host::Local { shared, core_name }) => {
                match call_session_tool(&shared.handle(), core_name, name, &args) {
                    Some(outcome) => outcome,
                    None => Err(format!("unknown tool: {name}")),
                }
            }
            // An attached session answers its own tools; forward the call frame
            // and hand back the result it produced.
            #[cfg(unix)]
            Some(Host::Attached { client }) => {
                return match client.request("tools/call", params.clone()) {
                    Ok(result) => result,
                    Err(error) => {
                        outcome_json(Err(format!("the attached session is gone: {error}")))
                    }
                };
            }
            None if name == "status" => text(idle_status_text()),
            None => Err(format!(
                "nothing loaded; call load_rom or attach first (tool: {name})"
            )),
        },
    };
    outcome_json(outcome)
}

/// Load a ROM through the factory and make it the server's active session.
fn load_rom(loaded: &mut Option<Host>, args: &Value) -> ToolOutcome {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("'path' (string) is required")?;
    let bytes = std::fs::read(path).map_err(|error| format!("failed to read {path}: {error}"))?;
    let options = LoadOptions {
        tv_standard: args
            .get("tv_standard")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..LoadOptions::default()
    };
    let path_ref = Path::new(path);
    let console = factory::create_console_with(path_ref, &bytes, &options)?
        .ok_or_else(|| format!("no core recognises {path}"))?;
    let debugger = console
        .into_debugger()
        .map_err(|_| "this system has no debugger backend".to_string())?;
    let core_name = factory::factory_for(path_ref, &bytes)
        .map(|factory| factory.name)
        .unwrap_or("unknown");
    let shared = SharedSession::spawn(debugger);
    let title = shared.handle().with_session(|s| s.game_title());
    *loaded = Some(Host::Local { shared, core_name });
    text(format!("loaded {core_name}: {title}"))
}

/// Attach to a session another process published, and drive it from here.
#[cfg(unix)]
fn attach(loaded: &mut Option<Host>, args: &Value) -> ToolOutcome {
    use missingno_session::attach::{AttachClient, discover};

    let client = match args.get("path").and_then(Value::as_str) {
        Some(path) => AttachClient::connect(std::path::Path::new(path))?,
        None => {
            let wanted = args.get("pid").and_then(Value::as_u64);
            let sessions = discover();
            let chosen = match wanted {
                Some(pid) => sessions
                    .into_iter()
                    .find(|session| u64::from(session.pid) == pid)
                    .ok_or_else(|| format!("no reachable session with pid {pid}"))?,
                None => match sessions.len() {
                    0 => return Err(NO_SESSIONS.into()),
                    1 => sessions.into_iter().next().expect("one session"),
                    // Which live machine to join is the agent's choice, never a
                    // guess made here.
                    _ => {
                        return Err(format!(
                            "several sessions are reachable; \
                                             give a pid:\n{}",
                            session_list(&sessions)
                        ));
                    }
                },
            };
            AttachClient::connect(&chosen.path)?
        }
    };
    let summary = client.info().summary();
    *loaded = Some(Host::Attached { client });
    text(format!("attached to {summary}"))
}

#[cfg(not(unix))]
fn attach(_loaded: &mut Option<Host>, _args: &Value) -> ToolOutcome {
    Err("attaching to a running session needs Unix domain sockets".into())
}

/// Unload the active machine — a loaded ROM or an attached session — returning
/// the server to idle. Detaching leaves the other process's session running.
fn eject(loaded: &mut Option<Host>) -> ToolOutcome {
    match loaded.take() {
        Some(host) => text(format!("ejected {}", host.description())),
        None => Err("nothing loaded".into()),
    }
}

const NO_SESSIONS: &str = "no running sessions are reachable (an app window publishes one only \
                           while it allows external debugger clients)";

#[cfg(unix)]
fn session_list(sessions: &[missingno_session::attach::SessionInfo]) -> String {
    sessions
        .iter()
        .map(|session| session.summary())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The idle server's status: what it can do, and which live sessions it can
/// reach right now.
fn idle_status_text() -> String {
    let mut body =
        "idle: nothing loaded. Call load_rom with a ROM path, or attach to a running session."
            .to_string();
    #[cfg(unix)]
    {
        let sessions = missingno_session::attach::discover();
        body.push_str("\n\n");
        if sessions.is_empty() {
            body.push_str(NO_SESSIONS);
        } else {
            body.push_str("reachable sessions:\n");
            body.push_str(&session_list(&sessions));
        }
    }
    body
}
