//! An MCP-over-stdio transport over a session's agent tool surface, for an agent
//! to drive the headless debugger as a tool server. The JSON-RPC loop and frames
//! are [`missingno_mcp_stdio`]'s; this module supplies what the server drives.
//!
//! stdout is the protocol channel: every log line goes to stderr. The tools
//! themselves belong to the session, not to this transport: the server adds only
//! the management tools that pick what it drives — a ROM it loads, or a session
//! another process published.

use std::io;
use std::path::Path;

use serde_json::{Value, json};

use missingno_core::launch::{LaunchOptionKind, LaunchValues};
use missingno_mcp_stdio::no_arguments;
use missingno_session::factory::{self, CoreFactory};
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
    Attached {
        client: missingno_session::attach::AttachClient,
    },
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

/// The server's state: what it drives, if anything.
struct Server {
    loaded: Option<Host>,
}

impl missingno_mcp_stdio::Server for Server {
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Loading, attaching, and ejecting swap the whole tool set.
    type ToolSetId = Option<String>;

    fn name(&self) -> String {
        match &self.loaded {
            Some(host) => format!("missingno-debugger ({})", host.description()),
            None => "missingno-debugger (idle)".to_string(),
        }
    }

    fn tool_set_id(&self) -> Option<String> {
        self.loaded.as_ref().map(Host::description)
    }

    fn list_tools(&mut self) -> Value {
        tools_list(&mut self.loaded)
    }

    fn call_tool(&mut self, params: &Value) -> Value {
        tools_call(&mut self.loaded, params)
    }
}

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

/// The stdio serve loop, over an optional loaded session. Every tool is a
/// [`Session`] call: the transport is Session-only by construction, with no
/// family-specific escape hatch.
fn run(loaded: Option<Host>) -> io::Result<()> {
    match &loaded {
        Some(host) => eprintln!("mcp: {} ready on stdio", host.description()),
        None => eprintln!("mcp: idle (no ROM loaded) ready on stdio"),
    }
    missingno_mcp_stdio::serve(&mut Server { loaded })
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
                      recognised from the file across all enabled cores. `options` sets the \
                      launch options the recognised core publishes, each left out to let the \
                      core resolve it: the Atari VCS takes `tv-standard` (ntsc/pal/secam), \
                      `board` (a cartridge board code such as F8, F6SC, E0), and `overdump` \
                      (boolean); the Game Boy family takes `runner` (dmg/cgb) and `boot-rom` \
                      (path to a boot ROM image). `tv_standard` is the older spelling of the \
                      VCS standard override."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "filesystem path to the ROM" },
                "options": {
                    "type": "object",
                    "description": "launch option id to value: a string for a choice or a file \
                                    path, true/false for a toggle",
                },
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
        input_schema: no_arguments(),
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
        input_schema: no_arguments(),
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
    let path_ref = Path::new(path);
    let factory = factory::factory_for(path_ref, &bytes)
        .ok_or_else(|| format!("no core recognises {path}"))?;
    let launch = launch_values(factory, &bytes, args)?;
    let console = (factory.create)(path_ref, &bytes, &launch).map_err(|error| error.to_string())?;
    let debugger = console.into_debugger();
    let core_name = factory.name;
    let shared = SharedSession::spawn(debugger);
    let title = shared.handle().with_session(|s| s.game_title());
    *loaded = Some(Host::Local { shared, core_name });
    text(format!("loaded {core_name}: {title}"))
}

/// The launch values a call names, read against the options the recognised core
/// publishes for this media: an option it does not publish, or a value of the
/// wrong shape, is an error rather than a setting quietly dropped.
fn launch_values(factory: &CoreFactory, rom: &[u8], args: &Value) -> Result<LaunchValues, String> {
    let mut launch = LaunchValues::default();
    // The VCS broadcast standard had a parameter of its own before the option
    // bag existed, and keeps answering to it.
    if let Some(standard) = args.get("tv_standard").and_then(Value::as_str) {
        launch.set_choice("tv-standard", standard);
    }
    let Some(options) = args.get("options") else {
        return Ok(launch);
    };
    let options = options
        .as_object()
        .ok_or("'options' must be an object of option id to value")?;
    let published = (factory.options)(rom);
    for (id, value) in options {
        let descriptor = published
            .iter()
            .find(|option| option.id == id)
            .ok_or_else(|| {
                let known: Vec<&str> = published.iter().map(|option| option.id).collect();
                match known.is_empty() {
                    true => format!("{} takes no launch options", factory.name),
                    false => format!(
                        "{} has no launch option '{id}'; it takes: {}",
                        factory.name,
                        known.join(", ")
                    ),
                }
            })?;
        match descriptor.kind {
            LaunchOptionKind::Choice { .. } => {
                let chosen = value
                    .as_str()
                    .ok_or_else(|| format!("launch option '{id}' takes a string"))?;
                launch.set_choice(id, chosen);
            }
            LaunchOptionKind::Toggle => {
                let flag = value
                    .as_bool()
                    .ok_or_else(|| format!("launch option '{id}' takes true or false"))?;
                launch.set_toggle(id, flag);
            }
            LaunchOptionKind::File { .. } => {
                let file = value
                    .as_str()
                    .ok_or_else(|| format!("launch option '{id}' takes a filesystem path"))?;
                let contents = std::fs::read(file)
                    .map_err(|error| format!("launch option '{id}': {file}: {error}"))?;
                launch.set_file(id, contents);
            }
        }
    }
    Ok(launch)
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
