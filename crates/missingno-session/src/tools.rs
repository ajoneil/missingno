//! The session's own agent tool surface: the tools a live session serves, and
//! the bodies that answer them. It speaks the MCP tool vocabulary — named tools
//! with JSON Schema arguments, results as text or embedded images — but links no
//! transport. The stdio server serves these for a local session and the attach
//! endpoint publishes the identical functions for a remote one, so both clients
//! see one surface.

use missingno_core::graphics::{GraphicsView, Object, ObjectTable, PaletteSet, TileAtlas};
use missingno_core::inspect::{
    BitTable, PairMatrix, PixelStrip, Register, Section, SectionBlock, SwatchRow, ValueStyle,
};
use serde_json::{Value, json};

use crate::request::{parse_control, parse_hex_arg, parse_watch_terms};
use crate::session::{Session, StopReason};
use crate::shared::SessionHandle;

/// Cap on a single `read_memory`, so a bad length can't allocate unbounded.
const MAX_MEMORY_LEN: u32 = 0x1000;
/// Default and cap for a `disassemble` window.
const DEFAULT_DISASM_COUNT: usize = 16;
const MAX_DISASM_COUNT: usize = 256;
/// Cap on instructions/dots run by a single stepping call.
const MAX_STEP_COUNT: usize = 1_000_000;
/// Cap on frames run by a single `step_frame`.
const MAX_FRAME_COUNT: usize = 3600;
/// Cap on sub-instruction ticks run by a single `step_tick`.
const MAX_TICK_COUNT: usize = 1_000_000;
/// One item of a tool result's content: agent-readable text, or an embedded
/// image (the resolved frame as a PNG).
pub enum Content {
    Text(String),
    Image { data: String, mime_type: String },
}

impl Content {
    fn to_json(&self) -> Value {
        match self {
            Content::Text(text) => json!({ "type": "text", "text": text }),
            Content::Image { data, mime_type } => {
                json!({ "type": "image", "data": data, "mimeType": mime_type })
            }
        }
    }
}

/// A tool's outcome: content on success, an error message rendered as an
/// `isError` text result on failure.
pub type ToolOutcome = Result<Vec<Content>, String>;

/// One advertised tool: its name, an agent-facing description, and the JSON
/// Schema for its arguments.
pub struct Tool {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
}

impl Tool {
    /// The tool's advertisement, as one item of a `tools/list` result.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}
/// The tools a live session serves every client — the surface the stdio server
/// advertises for a local session and an attach endpoint publishes for a remote
/// one. A plain-console session offers only the tools that need no debugger.
pub fn session_tools(handle: &SessionHandle) -> Vec<Tool> {
    let mut tools = machine_tools();
    if handle.is_debugger() {
        tools.extend(handle.with_session(|session| generic_tools(session)));
    }
    tools
}

/// [`session_tools`] as the `tools/list` result body.
pub fn session_tools_json(handle: &SessionHandle) -> Value {
    let tools: Vec<Value> = session_tools(handle).iter().map(Tool::to_json).collect();
    json!({ "tools": tools })
}

/// The tools answered by the session's command queue rather than by its debugger
/// — free-running control, input, and recording capture. They work on both
/// machine kinds, so a plain-console session still serves them.
fn machine_tools() -> Vec<Tool> {
    let empty = || json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let path = |what: &str| {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": what } },
            "required": ["path"],
        })
    };
    vec![
        Tool {
            name: "status",
            description: "The machine's core, title, program counter, frame count, last stop \
                          reason, and whether it is running or recording."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "reset",
            description: "Reset the console to power-on, finalizing any recording and dropping \
                          any replay."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "run",
            description: "Start the machine free-running at its native frame rate. Any client \
                          watching this session — including the app's own window — follows."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "pause",
            description: "Stop free-running and block until the machine has halted, so the next \
                          readout sees the paused core."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "set_control",
            description: "Drive a console control: control id (0-7 map to the Game Boy button \
                          order), pressed, or an analog axis 0.0-1.0. Captured into an active \
                          recording."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "control": { "type": "integer", "minimum": 0, "maximum": 255 },
                    "pressed": { "type": "boolean" },
                    "axis": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                },
                "required": ["control"],
            }),
        },
        Tool {
            name: "save_state",
            description: "Write the current machine state to a filesystem path as a save file."
                .into(),
            input_schema: path("save file to write"),
        },
        Tool {
            name: "load_state",
            description: "Restore the machine state from a save file, finalizing any recording \
                          and dropping any replay. Errors on a state for a different system or \
                          ROM, an unsupported version, or a corrupt file."
                .into(),
            input_schema: path("save file to read"),
        },
        Tool {
            name: "start_recording",
            description: "Begin capturing an input recording, written when it is stopped. Every \
                          control driven from any client is captured."
                .into(),
            input_schema: path("recording file to write"),
        },
        Tool {
            name: "stop_recording",
            description: "Finish and write the active recording.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "play_recording",
            description: "Replay a recording, driving the machine frame by frame so the playback \
                          is watchable. A checkpoint disagreement stops it and reports the frame."
                .into(),
            input_schema: path("recording file to read"),
        },
    ]
}
fn generic_tools(session: &Session) -> Vec<Tool> {
    let hex = || json!({ "type": "string", "description": "hex address, e.g. \"ff40\"" });
    let empty = || json!({ "type": "object", "properties": {}, "additionalProperties": false });

    let mut tools = vec![
        Tool {
            name: "read_registers",
            description: "Every register group, each value rendered in its natural style.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "read_memory",
            description: format!(
                "Hex dump of console memory from an address. Length 1..={MAX_MEMORY_LEN}."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": hex(),
                    "length": { "type": "integer", "minimum": 1, "maximum": MAX_MEMORY_LEN },
                },
                "required": ["address"],
            }),
        },
        Tool {
            name: "list_regions",
            description: "The CPU-visible memory map, named by role.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "list_symbols",
            description: "User debug-symbol labels loaded for this ROM.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "disassemble",
            description: format!(
                "Disassembly window: address (default pc), count (default {DEFAULT_DISASM_COUNT}, \
                 max {MAX_DISASM_COUNT})."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": hex(),
                    "count": { "type": "integer", "minimum": 1, "maximum": MAX_DISASM_COUNT },
                },
            }),
        },
        Tool {
            name: "describe_machine",
            description: "The full machine-state sidebar as text: registers, bit tables, sweeps, \
                          and pixel strips."
                .into(),
            input_schema: empty(),
        },
        Tool {
            name: "get_waveforms",
            description:
                "Each sound channel's captured DAC waveform as a text sparkline, with its \
                          rate and whether it is driving. Enables capture if it was off; steps \
                          nothing, so call again after stepping to see the window fill."
                    .into(),
            input_schema: empty(),
        },
        Tool {
            name: "get_tiles",
            description: "Decoded tile atlas. Default: a PNG image of the whole atlas grid \
                          (greyscale) plus a summary line. With `tile`: that one tile's shade \
                          glyph grid and raw palette indices as text. `atlas` selects which \
                          atlas (default 0). Enables graphics capture if it was off."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "atlas": { "type": "integer", "minimum": 0, "description": "atlas index (default 0)" },
                    "tile": { "type": "integer", "minimum": 0, "description": "a single tile to detail as glyphs" },
                },
            }),
        },
        Tool {
            name: "get_objects",
            description: "The object/sprite table. Default: a text table (index, x, y, tile, \
                          palette, bank, flips, priority, on-screen). With `object`: that \
                          entry's fields and its composed sprite as a shade glyph grid. \
                          Enables graphics capture if it was off."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "object": { "type": "integer", "minimum": 0, "description": "a single object to detail" },
                },
            }),
        },
        Tool {
            name: "step",
            description: format!(
                "Execute instructions (count, default 1, max {MAX_STEP_COUNT}); stops early on a \
                 breakpoint or watch."
            ),
            input_schema: count_schema(MAX_STEP_COUNT),
        },
        Tool {
            name: "step_over",
            description: "Execute one instruction, stepping over a call.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "step_frame",
            description: format!(
                "Run whole frames (count, default 1, max {MAX_FRAME_COUNT}); stops early on a \
                 breakpoint or watch."
            ),
            input_schema: count_schema(MAX_FRAME_COUNT),
        },
        Tool {
            name: "set_breakpoint",
            description: "Set a PC breakpoint at a hex address.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "address": hex() },
                "required": ["address"],
            }),
        },
        Tool {
            name: "clear_breakpoint",
            description: "Clear the PC breakpoint at a hex address.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "address": hex() },
                "required": ["address"],
            }),
        },
        Tool {
            name: "list_breakpoints",
            description: "The set PC breakpoints.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "add_watch",
            description: watch_description(session, "Add"),
            input_schema: watch_schema(),
        },
        Tool {
            name: "remove_watch",
            description: watch_description(session, "Remove"),
            input_schema: watch_schema(),
        },
        Tool {
            name: "list_watches",
            description: "The active watches.".into(),
            input_schema: empty(),
        },
        Tool {
            name: "get_frame",
            description: "The current resolved screen as a PNG image.".into(),
            input_schema: empty(),
        },
    ];

    // Sub-instruction stepping is advertised only when the core names a tick
    // finer than an instruction (a Game Boy dot, a VCS colour clock).
    tools.extend(step_tick_tool(session.tick_name()));

    tools
}

/// The sub-instruction stepping tool, present only when the core names a tick.
fn step_tick_tool(tick_name: Option<&str>) -> Option<Tool> {
    let tick = tick_name?;
    Some(Tool {
        name: "step_tick",
        description: format!(
            "Advance the console by sub-instruction ticks (one {tick} each); count \
             default 1, max {MAX_TICK_COUNT}. Reports the resulting pc and video position."
        ),
        input_schema: count_schema(MAX_TICK_COUNT),
    })
}

fn count_schema(max: usize) -> Value {
    json!({
        "type": "object",
        "properties": { "count": { "type": "integer", "minimum": 1, "maximum": max } },
    })
}

fn watch_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "key": { "type": "string", "description": "watchable key" },
            "address": { "type": "string", "description": "hex address, if the key takes one" },
            "value": { "type": "integer", "description": "value, if the key takes one" },
            "terms": {
                "type": "array",
                "description": "a multi-term conjunction; alternative to a single key/address/value",
                "items": { "type": "object" },
            },
        },
    })
}

fn watch_description(session: &Session, verb: &str) -> String {
    use missingno_core::inspect::WatchParam;
    let keys: Vec<String> = session
        .watchables()
        .iter()
        .map(|watchable| {
            let shape = match watchable.param {
                WatchParam::None => "no params",
                WatchParam::Address => "address",
                WatchParam::Value { .. } => "value",
                WatchParam::AddressValue => "address+value",
            };
            format!("{} ({shape})", watchable.key)
        })
        .collect();
    if keys.is_empty() {
        format!("{verb} a watch. This core exposes no watchables.")
    } else {
        format!("{verb} a watch. Keys for this core: {}.", keys.join(", "))
    }
}
/// Answer one tool call against a live session, whichever client asked. The
/// machine-level tools are answered by the session's command queue; everything
/// else runs against the owned debugger. `None` names a tool this session has no
/// answer for.
pub fn call_session_tool(
    handle: &SessionHandle,
    core_name: &str,
    name: &str,
    args: &Value,
) -> Option<ToolOutcome> {
    let outcome = match name {
        "run" => {
            handle.run();
            status_report(handle, core_name)
        }
        "pause" => {
            handle.pause();
            status_report(handle, core_name)
        }
        "set_control" => set_control(handle, args),
        "save_state" => path_arg(args)
            .and_then(|path| handle.save_state(path))
            .map(|()| vec![Content::Text("state saved".into())]),
        "load_state" => path_arg(args)
            .and_then(|path| handle.load_state(path))
            .and_then(|()| status_report(handle, core_name)),
        "start_recording" => path_arg(args)
            .and_then(|path| handle.start_recording(path))
            .map(|()| vec![Content::Text("recording".into())]),
        "stop_recording" => handle
            .stop_recording()
            .map(|()| vec![Content::Text("recording written".into())]),
        "play_recording" => path_arg(args)
            .and_then(|path| handle.play_recording(path))
            .map(|()| vec![Content::Text("replaying".into())]),
        "reset" => {
            handle.reset();
            status_report(handle, core_name)
        }
        // The run/recording state lives on the session, not in the debugger, so
        // the status line is completed here.
        "status" => status_report(handle, core_name),
        _ if !handle.is_debugger() => {
            return Some(Err(format!(
                "this session hosts no debugger, so it has no {name}; it serves run, pause, \
                 set_control, and recording"
            )));
        }
        _ => {
            let tool = name.to_string();
            let arguments = args.clone();
            return handle.with_session(move |session| call_generic(session, &tool, &arguments));
        }
    };
    Some(outcome)
}

/// [`call_session_tool`] over a whole `tools/call` params object, as the result
/// body — what an attach endpoint answers a forwarded call frame with.
pub fn call_session_tool_json(handle: &SessionHandle, core_name: &str, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    outcome_json(
        call_session_tool(handle, core_name, name, &args)
            .unwrap_or_else(|| Err(format!("unknown tool: {name}"))),
    )
}

/// The status line for either machine kind: the debugger's own view where there
/// is one, and the session's run/recording state either way.
fn status_report(handle: &SessionHandle, core_name: &str) -> ToolOutcome {
    let running = handle.is_running();
    let recording = handle.is_recording();
    let core = core_name.to_string();
    let body = if handle.is_debugger() {
        handle.with_session(move |session| status_text(session, &core))
    } else {
        format!("core: {core}\n(a plain console: no debugger surface)")
    };
    text(format!(
        "{body}\nrunning: {running}\nrecording: {recording}"
    ))
}

/// The `path` argument every file-taking tool requires.
fn path_arg(args: &Value) -> Result<std::path::PathBuf, String> {
    args.get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "'path' (string) is required".to_string())
}
/// A tool outcome as the `tools/call` result body a transport emits.
pub fn outcome_json(outcome: ToolOutcome) -> Value {
    match outcome {
        Ok(content) => json!({
            "content": content.iter().map(Content::to_json).collect::<Vec<_>>(),
            "isError": false,
        }),
        Err(message) => json!({
            "content": [ { "type": "text", "text": message } ],
            "isError": true,
        }),
    }
}

/// Wrap a plain text body as a single-item successful outcome.
pub fn text(body: impl Into<String>) -> ToolOutcome {
    Ok(vec![Content::Text(body.into())])
}
fn call_generic(session: &mut Session, name: &str, args: &Value) -> Option<ToolOutcome> {
    let outcome = match name {
        "read_registers" => text(registers_text(session)),
        "read_memory" => read_memory(session, args),
        "list_regions" => text(regions_text(session)),
        "list_symbols" => text(symbols_text(session)),
        "disassemble" => disassemble(session, args),
        "describe_machine" => text(describe_machine(session)),
        "get_waveforms" => text(waveforms_text(session)),
        "get_tiles" => get_tiles(session, args),
        "get_objects" => get_objects(session, args),
        "step" => stepping(session, args, Stepping::Instruction),
        "step_over" => {
            let stop = session.step_over();
            text(step_report(session, &stop, 1))
        }
        "step_frame" => stepping(session, args, Stepping::Frame),
        "step_tick" => step_tick(session, args),
        "set_breakpoint" => breakpoint(session, args, true),
        "clear_breakpoint" => breakpoint(session, args, false),
        "list_breakpoints" => text(breakpoints_text(session)),
        "add_watch" => watch_edit(session, args, true),
        "remove_watch" => watch_edit(session, args, false),
        "list_watches" => text(watches_text(session)),
        "get_frame" => get_frame(session),
        _ => return None,
    };
    Some(outcome)
}

// --- generic tool bodies ------------------------------------------------------

/// The display device line for the status view: the technology and the panel
/// class or broadcast standard the core states.
fn display_line(video: &missingno_core::video::DisplayTechnology) -> String {
    use missingno_core::video::DisplayTechnology;
    match video {
        DisplayTechnology::Lcd { native, panel, .. } => {
            format!("LCD ({}) {}x{}", panel.description(), native.0, native.1)
        }
        DisplayTechnology::Crt { standard, .. } => format!("CRT {}", standard.name()),
    }
}

fn status_text(session: &Session, core_name: &str) -> String {
    let display = display_line(&session.video_out());
    format!(
        "core: {core_name}\ntitle: {title}\ndisplay: {display}\npc: {pc:04x}\nframe: {frame}\n\
         stop: {stop}",
        title = session.game_title(),
        pc = session.pc(),
        frame = session.frame(),
        stop = stop_text(session.last_stop()),
    )
}

fn stop_text(stop: &StopReason) -> String {
    match stop {
        StopReason::Completed => "completed".into(),
        StopReason::Breakpoint => "breakpoint".into(),
        StopReason::BudgetExhausted => "budget-exhausted".into(),
        StopReason::Watch(watch) => {
            let terms: Vec<String> = watch
                .terms
                .iter()
                .map(|term| {
                    let mut parts = term.key.clone();
                    if let Some(address) = term.address {
                        parts.push_str(&format!(" @{address:x}"));
                    }
                    if let Some(value) = term.value {
                        parts.push_str(&format!(" ={value}"));
                    }
                    parts
                })
                .collect();
            format!("watch[{}]", terms.join(" & "))
        }
    }
}

fn registers_text(session: &Session) -> String {
    let mut out = String::new();
    for group in session.register_groups() {
        out.push_str(group.name);
        out.push_str(":\n");
        for register in &group.registers {
            out.push_str(&format!(
                "  {} = {}\n",
                register.name,
                render_register(register)
            ));
        }
    }
    if out.is_empty() {
        out.push_str("(no registers)");
    }
    out.trim_end().to_string()
}

fn render_register(register: &Register) -> String {
    match register.style {
        ValueStyle::Hex => {
            let width = (register.bits as usize).div_ceil(4).max(1);
            format!("{:0width$x}", register.value, width = width)
        }
        ValueStyle::Dec => register.value.to_string(),
        ValueStyle::Bool => (register.value != 0).to_string(),
        ValueStyle::Flags(names) => {
            let parts: Vec<String> = names
                .iter()
                .map(|flag| {
                    let set = (register.value >> flag.bit) & 1 != 0;
                    format!("{}{}", flag.name, if set { "+" } else { "-" })
                })
                .collect();
            format!("[{}]", parts.join(" "))
        }
    }
}

fn read_memory(session: &Session, args: &Value) -> ToolOutcome {
    let address = parse_hex_arg(args, "address")?;
    let length = match args.get("length") {
        None | Some(Value::Null) => 1,
        Some(value) => {
            let length = value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("length must be a non-negative integer")?;
            if !(1..=MAX_MEMORY_LEN).contains(&length) {
                return Err(format!("length must be 1..={MAX_MEMORY_LEN}"));
            }
            length
        }
    };
    let bytes = session.memory(address, length);
    Ok(vec![Content::Text(hex_dump(address, &bytes))])
}

fn hex_dump(base: u32, bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let addr = base.wrapping_add(row as u32 * 16);
        let hex: Vec<String> = chunk.iter().map(|byte| format!("{byte:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&byte| {
                if (0x20..0x7f).contains(&byte) {
                    byte as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!("{addr:04x}  {:<48}  {ascii}\n", hex.join(" ")));
    }
    out.trim_end().to_string()
}

fn regions_text(session: &Session) -> String {
    let regions = session.memory_regions();
    if regions.is_empty() {
        return "(no named regions)".into();
    }
    regions
        .iter()
        .map(|region| {
            format!(
                "{:04x}..{:04x}  {}",
                region.start,
                region.start.wrapping_add(region.len),
                region.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn symbols_text(session: &Session) -> String {
    let symbols = session.symbols();
    let entries = symbols.user_symbols();
    if entries.is_empty() {
        return "(no user symbols)".into();
    }
    entries
        .iter()
        .map(|symbol| {
            format!(
                "{:02x}:{:04x}  {}",
                symbol.bank, symbol.address, symbol.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn disassemble(session: &Session, args: &Value) -> ToolOutcome {
    let at = match args.get("address") {
        None | Some(Value::Null) => session.pc(),
        Some(_) => parse_hex_arg(args, "address")?,
    };
    let count = match args.get("count") {
        None | Some(Value::Null) => DEFAULT_DISASM_COUNT,
        Some(value) => value
            .as_u64()
            .map(|n| (n as usize).clamp(1, MAX_DISASM_COUNT))
            .ok_or("count must be an integer")?,
    };
    let lines = session.disassembly(at, count)?;
    let body = lines
        .iter()
        .map(|line| {
            let bytes: Vec<String> = line
                .bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if line.is_data {
                format!(
                    "{:04x}  {:<9}  db ${}",
                    line.address,
                    bytes.join(" "),
                    bytes.join("")
                )
            } else {
                format!(
                    "{:04x}  {:<9}  {}",
                    line.address,
                    bytes.join(" "),
                    line.text
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text(body)
}

enum Stepping {
    Instruction,
    Frame,
}

fn stepping(session: &mut Session, args: &Value, kind: Stepping) -> ToolOutcome {
    let cap = match kind {
        Stepping::Instruction => MAX_STEP_COUNT,
        Stepping::Frame => MAX_FRAME_COUNT,
    };
    let count = match args.get("count") {
        None | Some(Value::Null) => 1,
        Some(value) => value
            .as_u64()
            .map(|n| (n as usize).clamp(1, cap))
            .ok_or("count must be an integer")?,
    };
    let mut stop = StopReason::Completed;
    let mut ran = 0;
    for _ in 0..count {
        stop = match kind {
            Stepping::Instruction => session.step(),
            Stepping::Frame => session.step_frame(),
        };
        ran += 1;
        if matches!(stop, StopReason::Breakpoint | StopReason::Watch(_)) {
            break;
        }
    }
    text(step_report(session, &stop, ran))
}

fn step_report(session: &Session, stop: &StopReason, ran: usize) -> String {
    format!(
        "ran: {ran}\npc: {:04x}\nframe: {}\nstop: {}",
        session.pc(),
        session.frame(),
        stop_text(stop)
    )
}

fn step_tick(session: &mut Session, args: &Value) -> ToolOutcome {
    let Some(tick) = session.tick_name() else {
        return Err("this core has no sub-instruction stepping".into());
    };
    let count = match args.get("count") {
        None | Some(Value::Null) => 1,
        Some(value) => value
            .as_u64()
            .map(|n| (n as usize).clamp(1, MAX_TICK_COUNT))
            .ok_or("count must be an integer")?,
    };
    for _ in 0..count {
        session.step_tick();
    }
    text(tick_report(session, tick, count))
}

fn tick_report(session: &Session, tick: &str, ran: usize) -> String {
    let status = session.running_status();
    let plural = if ran == 1 { "" } else { "s" };
    format!(
        "ran: {ran} {tick}{plural}\npc: {:04x}\n{}: {}",
        status.pc, status.video_label, status.video_summary,
    )
}

fn breakpoint(session: &mut Session, args: &Value, set: bool) -> ToolOutcome {
    let address = parse_hex_arg(args, "address")?;
    if set {
        session.set_breakpoint(address)?;
        text(format!("set breakpoint at {address:04x}"))
    } else {
        session.clear_breakpoint(address);
        text(format!("cleared breakpoint at {address:04x}"))
    }
}

fn breakpoints_text(session: &Session) -> String {
    let breakpoints = session.breakpoints();
    if breakpoints.is_empty() {
        return "(no breakpoints)".into();
    }
    breakpoints
        .iter()
        .map(|address| format!("{address:04x}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn watch_edit(session: &mut Session, args: &Value, add: bool) -> ToolOutcome {
    let terms = parse_watch_terms(args)?;
    let result = if add {
        session.add_watch(terms)
    } else {
        session.remove_watch(terms)
    };
    let watch = result?;
    let verb = if add { "added" } else { "removed" };
    text(format!(
        "{verb} watch: {}",
        stop_text(&StopReason::Watch(watch))
    ))
}

fn watches_text(session: &Session) -> String {
    let watches = session.watches();
    if watches.is_empty() {
        return "(no watches)".into();
    }
    watches
        .iter()
        .map(|watch| stop_text(&StopReason::Watch(watch.clone())))
        .collect::<Vec<_>>()
        .join("\n")
}

fn get_frame(session: &Session) -> ToolOutcome {
    let frame = session.frame_rgba();
    let png = encode_png(frame.width, frame.height, &frame.pixels)?;
    Ok(vec![Content::Image {
        data: base64_encode(&png),
        mime_type: "image/png".into(),
    }])
}

/// Driven through the session's command queue rather than the debugger, so an
/// agent's input lands in an active recording exactly as a user's does.
fn set_control(handle: &SessionHandle, args: &Value) -> ToolOutcome {
    let (control, input) = parse_control(args)?;
    handle.set_control(control, input);
    text(format!("control {} set", control.0))
}

// --- get_waveforms ------------------------------------------------------------

/// Sparkline glyphs from lowest to highest level.
const SPARK_BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Column budget a waveform sparkline downsamples to.
const SPARK_WIDTH: usize = 48;

fn waveforms_text(session: &mut Session) -> String {
    if session.channel_waves().is_none() {
        session.set_wave_capture(true);
    }
    let Some(waves) = session.channel_waves() else {
        return "(this core captures no waveforms)".into();
    };
    let mut out = String::new();
    for wave in &waves {
        let max = ((1u32 << wave.depth_bits.min(31)) - 1).max(1) as u8;
        let spark = sparkline(&wave.levels, max, SPARK_WIDTH);
        out.push_str(&format!(
            "{} @ {} Hz  {}\n  {}\n",
            wave.label,
            wave.rate,
            if wave.active { "driving" } else { "idle" },
            if spark.is_empty() {
                "(no samples yet)".to_string()
            } else {
                spark
            },
        ));
    }
    out.trim_end().to_string()
}

/// Reduce `levels` to at most `width` sparkline glyphs, each the peak code of
/// the samples in its column so a spike is not aliased away.
fn sparkline(levels: &[u8], max_code: u8, width: usize) -> String {
    let n = levels.len();
    if n == 0 || width == 0 {
        return String::new();
    }
    let cols = width.min(n);
    (0..cols)
        .map(|col| {
            let start = col * n / cols;
            let end = ((col + 1) * n / cols).max(start + 1).min(n);
            let peak = levels[start..end].iter().copied().max().unwrap_or(0);
            let step = (peak as usize * (SPARK_BARS.len() - 1)) / max_code.max(1) as usize;
            SPARK_BARS[step.min(SPARK_BARS.len() - 1)]
        })
        .collect()
}

// --- get_tiles / get_objects --------------------------------------------------

/// Tiles per row in the atlas PNG, matching the debugger's Tiles pane.
const ATLAS_COLUMNS: usize = 16;
/// Shade glyphs for a 2bpp tile, lightest to darkest.
const TILE_GLYPHS: [char; 4] = ['·', '░', '▒', '▓'];

/// The decoded graphics surfaces, enabling capture first if it was off — the
/// `get_waveforms` pattern. `None` when the core exposes none.
fn ensure_graphics(session: &mut Session) -> Option<GraphicsView> {
    // Enabling capture is a no-op when already on, so one decode suffices.
    session.set_graphics_capture(true);
    session.graphics()
}

fn get_tiles(session: &mut Session, args: &Value) -> ToolOutcome {
    let Some(graphics) = ensure_graphics(session) else {
        return Err("this core exposes no graphics surfaces".into());
    };
    let atlas_index = args.get("atlas").and_then(Value::as_u64).unwrap_or(0) as usize;
    let Some(atlas) = graphics.atlases.get(atlas_index) else {
        return Err(format!(
            "no atlas {atlas_index} (have {})",
            graphics.atlases.len()
        ));
    };
    match args.get("tile").and_then(Value::as_u64) {
        Some(tile) => tile_detail(atlas, tile as usize),
        None => atlas_survey(atlas),
    }
}

/// The whole atlas as a greyscale PNG image block, with a summary line. The PNG
/// stays a plain 16-wide grid; the region grouping rides in the text summary.
fn atlas_survey(atlas: &TileAtlas) -> ToolOutcome {
    let (width, height, pixels) = atlas_pixels(atlas, ATLAS_COLUMNS);
    let png = encode_png(width, height, &pixels)?;
    Ok(vec![
        Content::Text(atlas_summary(atlas)),
        Content::Image {
            data: base64_encode(&png),
            mime_type: "image/png".into(),
        },
    ])
}

/// The survey text: the atlas header line plus one line per named tile region.
fn atlas_summary(atlas: &TileAtlas) -> String {
    let mut summary = format!(
        "{}: {} tiles, {}×{}, {}bpp, {}",
        atlas.label,
        atlas.tile_count(),
        atlas.tile_width,
        atlas.tile_height,
        atlas.depth_bits,
        palette_set_name(&atlas.palettes),
    );
    if !atlas.regions.is_empty() {
        summary.push_str("\nregions:");
        for region in &atlas.regions {
            let last = region.start + region.len - 1;
            summary.push_str(&format!(
                "\n  {}: tiles {}-{}",
                region.label, region.start, last
            ));
            if let Some(help) = region.help {
                summary.push_str(&format!(" ({help})"));
            }
        }
    }
    summary
}

/// One tile as a shade glyph grid over its raw palette indices.
fn tile_detail(atlas: &TileAtlas, tile: usize) -> ToolOutcome {
    if tile >= atlas.tile_count() {
        return Err(format!("no tile {tile} (atlas has {})", atlas.tile_count()));
    }
    let region = atlas
        .region_of(tile)
        .map(|region| format!(" ({})", region.label))
        .unwrap_or_default();
    let mut out = format!("tile {tile} of {}{region}:\n", atlas.label);
    for y in 0..atlas.tile_height {
        for x in 0..atlas.tile_width {
            out.push(tile_glyph(atlas.pixel(tile, x, y).unwrap_or(0)));
        }
        out.push('\n');
    }
    out.push_str("indices:\n");
    for y in 0..atlas.tile_height {
        let row: Vec<String> = (0..atlas.tile_width)
            .map(|x| atlas.pixel(tile, x, y).unwrap_or(0).to_string())
            .collect();
        out.push_str(&format!("{}\n", row.join(" ")));
    }
    text(out.trim_end())
}

/// Greyscale RGBA for the whole atlas, `columns` tiles wide.
fn atlas_pixels(atlas: &TileAtlas, columns: usize) -> (u32, u32, Vec<u8>) {
    let tile_w = atlas.tile_width as usize;
    let tile_h = atlas.tile_height as usize;
    let columns = columns.max(1);
    let rows = atlas.tile_count().div_ceil(columns).max(1);
    let width = (columns * tile_w) as u32;
    let height = (rows * tile_h) as u32;

    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for tile_row in 0..rows {
        for pixel_y in 0..tile_h {
            for tile_col in 0..columns {
                let tile = tile_row * columns + tile_col;
                for pixel_x in 0..tile_w {
                    let index = atlas.pixel(tile, pixel_x as u8, pixel_y as u8).unwrap_or(0);
                    let grey = shade(index, atlas.depth_bits);
                    pixels.extend_from_slice(&[grey, grey, grey, 255]);
                }
            }
        }
    }
    (width, height, pixels)
}

/// A palette index as a greyscale level, 0 lightest — the classic Game Boy
/// arrangement, family-agnostic here since no palette is chosen.
fn shade(index: u8, depth_bits: u8) -> u8 {
    let max = ((1u32 << depth_bits.min(8)) - 1).max(1);
    let index = (index as u32).min(max);
    (255 - index * 255 / max) as u8
}

fn tile_glyph(index: u8) -> char {
    TILE_GLYPHS[(index as usize).min(TILE_GLYPHS.len() - 1)]
}

fn palette_set_name(set: &PaletteSet) -> &'static str {
    match set {
        PaletteSet::FrontendShades => "frontend shades",
        PaletteSet::Owned(_) => "core palettes",
    }
}

fn get_objects(session: &mut Session, args: &Value) -> ToolOutcome {
    let Some(graphics) = ensure_graphics(session) else {
        return Err("this core exposes no graphics surfaces".into());
    };
    let Some(table) = &graphics.objects else {
        return Err("this core exposes no object table".into());
    };
    match args.get("object").and_then(Value::as_u64) {
        Some(object) => object_detail(&graphics, table, object as usize),
        None => text(objects_table(table)),
    }
}

/// The object table as text: one row per entry.
fn objects_table(table: &ObjectTable) -> String {
    let mut out = format!(
        "{}: {} objects, 8×{} sprites\n",
        table.label,
        table.objects.len(),
        table.object_height
    );
    out.push_str("idx    x    y  tile  pal bank flip pri  on\n");
    for object in &table.objects {
        out.push_str(&format!(
            "{:>3} {:>4} {:>4} {:>5}  {:>3} {:>4}  {:>2} {:>3}  {}\n",
            object.index,
            object.x,
            object.y,
            object.tile,
            opt(object.palette),
            opt(object.bank),
            flips(object.flip_x, object.flip_y),
            if object.priority { "beh" } else { "abv" },
            if object.on_screen { "yes" } else { "no" },
        ));
    }
    out.trim_end().to_string()
}

/// One object's fields plus its composed sprite as a shade glyph grid.
fn object_detail(graphics: &GraphicsView, table: &ObjectTable, index: usize) -> ToolOutcome {
    let Some(object) = table.objects.get(index) else {
        return Err(format!(
            "no object {index} (table has {})",
            table.objects.len()
        ));
    };
    let mut out = format!(
        "object {}:\n  x={} y={} tile={} palette={} bank={} flip_x={} flip_y={} \
         priority={} on_screen={}\n",
        object.index,
        object.x,
        object.y,
        object.tile,
        opt(object.palette),
        opt(object.bank),
        object.flip_x,
        object.flip_y,
        if object.priority { "behind" } else { "above" },
        object.on_screen,
    );
    match graphics
        .atlases
        .get(object.bank.unwrap_or(table.atlas) as usize)
    {
        Some(atlas) => {
            out.push_str(&format!("sprite (8×{}):\n", table.object_height));
            out.push_str(&object_glyphs(atlas, object, table.object_height));
        }
        None => out.push_str("(pattern atlas unavailable)\n"),
    }
    text(out.trim_end())
}

/// Compose an object's tile(s) — an 8×16 two-tile stack where `object_height`
/// exceeds one tile — into a shade glyph grid, flips applied.
fn object_glyphs(atlas: &TileAtlas, object: &Object, object_height: u8) -> String {
    let tile_w = atlas.tile_width;
    let tile_h = atlas.tile_height;
    let slots: Vec<u16> = if object_height > tile_h {
        let top = object.tile & !1;
        let bottom = object.tile | 1;
        if object.flip_y {
            vec![bottom, top]
        } else {
            vec![top, bottom]
        }
    } else {
        vec![object.tile]
    };

    let mut out = String::new();
    for slot in slots {
        for y in 0..tile_h {
            for x in 0..tile_w {
                let sx = if object.flip_x { tile_w - 1 - x } else { x };
                let sy = if object.flip_y { tile_h - 1 - y } else { y };
                out.push(tile_glyph(atlas.pixel(slot as usize, sx, sy).unwrap_or(0)));
            }
            out.push('\n');
        }
    }
    out
}

fn opt(value: Option<u8>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".into())
}

fn flips(flip_x: bool, flip_y: bool) -> String {
    format!(
        "{}{}",
        if flip_x { "x" } else { "-" },
        if flip_y { "y" } else { "-" }
    )
}

// --- describe_machine ---------------------------------------------------------

/// The empty glyph for a pixel-strip cell.
const CELL_EMPTY: char = '·';
/// A dim/light pixel-strip cell.
const CELL_LIGHT: char = '░';
/// A filled pixel-strip cell.
const CELL_FILLED: char = '▓';

fn describe_machine(session: &Session) -> String {
    let mut out = String::new();
    for section in session.sidebar_sections() {
        render_section_header(&section, &mut out);
        for block in &section.blocks {
            render_block(block, &mut out);
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        "(no machine-state sections)".into()
    } else {
        out.trim_end().to_string()
    }
}

fn render_section_header(section: &Section, out: &mut String) {
    out.push_str(&format!("== {} ==", section.name));
    if !section.summary.is_empty() {
        out.push_str(&format!("  {}", section.summary));
    }
    if let Some(detail) = &section.detail {
        out.push_str(&format!("  [{}]", detail.text));
    }
    if let Some(active) = section.active {
        out.push_str(if active { "  ●" } else { "  ○" });
    }
    out.push('\n');
}

fn render_block(block: &SectionBlock, out: &mut String) {
    match block {
        SectionBlock::Registers(group) => {
            let values: Vec<String> = group
                .registers
                .iter()
                .map(|register| format!("{}={}", register.name, render_register(register)))
                .collect();
            out.push_str(&format!("  {}: {}\n", group.name, values.join(" ")));
        }
        SectionBlock::Pairs(pairs) => {
            for pair in pairs {
                let width = ((pair.high.bits + pair.low.bits) as usize)
                    .div_ceil(4)
                    .max(1);
                out.push_str(&format!(
                    "  {}{}={:0width$x} ({}={} {}={})\n",
                    pair.high.name,
                    pair.low.name,
                    pair.combined(),
                    pair.high.name,
                    render_register(&pair.high),
                    pair.low.name,
                    render_register(&pair.low),
                    width = width,
                ));
            }
        }
        SectionBlock::Pointers(pointers) => {
            for pointer in pointers {
                let mark = match pointer.active {
                    Some(true) => " (running)",
                    Some(false) => " (stalled)",
                    None => "",
                };
                out.push_str(&format!(
                    "  {}={}{}\n",
                    pointer.register.name,
                    render_register(&pointer.register),
                    mark
                ));
            }
        }
        SectionBlock::Table(table) => render_table(table, out),
        SectionBlock::Relations(matrix) => render_relations(matrix, out),
        SectionBlock::Rows(rows) => {
            for row in rows {
                let pip = match row.active {
                    Some(true) => "● ",
                    Some(false) => "○ ",
                    None => "",
                };
                if row.value.is_empty() {
                    out.push_str(&format!("  {pip}{}\n", row.label));
                } else {
                    out.push_str(&format!("  {pip}{}: {}\n", row.label, row.value));
                }
            }
        }
        SectionBlock::Sweeps(sweeps) => {
            for sweep in sweeps {
                let zone = sweep
                    .zone_at(sweep.value)
                    .map(|zone| format!(" ({})", zone.name))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "  {} {}/{}{}\n",
                    sweep.label, sweep.value, sweep.end, zone
                ));
            }
        }
        SectionBlock::Swatches(swatches) => {
            for swatch in swatches {
                match swatch {
                    SwatchRow::Shades { label, packed } => {
                        out.push_str(&format!("  {label}: {packed:02x}\n"));
                    }
                    SwatchRow::Colors { label, colors } => {
                        let cells: Vec<String> = colors
                            .iter()
                            .map(|c| match c.raw {
                                Some(raw) => format!("${raw:04X}"),
                                None => {
                                    format!("#{:02x}{:02x}{:02x}", c.color.r, c.color.g, c.color.b)
                                }
                            })
                            .collect();
                        out.push_str(&format!("  {label}: {}\n", cells.join(" ")));
                    }
                }
            }
        }
        SectionBlock::Pixels(strips) => {
            for strip in strips {
                render_pixel_strip(strip, out);
            }
        }
        SectionBlock::Rule => out.push_str("  --------\n"),
    }
}

fn render_table(table: &BitTable, out: &mut String) {
    let header: Vec<&str> = table.columns.iter().map(|column| column.name).collect();
    let corner = table
        .corner
        .map(|flag| format!("  [{}:{}]", flag.name, if flag.active { "1" } else { "0" }))
        .unwrap_or_default();
    out.push_str(&format!("  {: <10}{}{corner}\n", "", header.join(" ")));
    for row in &table.rows {
        let bits: Vec<String> = row
            .bits
            .iter()
            .zip(&table.columns)
            .map(|(&bit, column)| {
                let width = column.name.len().max(1);
                format!("{:^width$}", if bit { "1" } else { "0" }, width = width)
            })
            .collect();
        out.push_str(&format!("  {: <10}{}\n", row.name, bits.join(" ")));
    }
}

fn render_relations(matrix: &PairMatrix, out: &mut String) {
    let n = matrix.entities.len();
    if n < 2 {
        return;
    }
    // Labels on the diagonal head their column and end their row, so both axes
    // read in the same order and every cell sits beside both of its labels.
    let column_width = matrix
        .entities
        .iter()
        .map(|name| name.len())
        .max()
        .unwrap_or(1);
    for row in 0..n {
        let mut cells: Vec<String> = (0..row)
            .map(|col| {
                let glyph = if matrix.cell(col, row).set {
                    "●"
                } else {
                    "·"
                };
                format!("{glyph:^column_width$}")
            })
            .collect();
        cells.push(matrix.entities[row].to_string());
        out.push_str(&format!("  {}\n", cells.join(" ")));
    }
}

fn render_pixel_strip(strip: &PixelStrip, out: &mut String) {
    let (label, glyphs) = match strip {
        PixelStrip::Shades { label, cells, .. } => {
            let glyphs: String = cells
                .iter()
                .map(|cell| match cell {
                    None => CELL_EMPTY,
                    Some(0) => CELL_LIGHT,
                    Some(_) => CELL_FILLED,
                })
                .collect();
            (label.to_string(), glyphs)
        }
        PixelStrip::Colors { label, cells, .. } => {
            let glyphs: String = cells
                .iter()
                .map(|cell| {
                    if cell.is_some() {
                        CELL_FILLED
                    } else {
                        CELL_EMPTY
                    }
                })
                .collect();
            (label.clone(), glyphs)
        }
        PixelStrip::Bits { label, cells, .. } => {
            let glyphs: String = cells
                .iter()
                .map(|&bit| if bit { CELL_FILLED } else { CELL_EMPTY })
                .collect();
            (label.to_string(), glyphs)
        }
    };
    out.push_str(&format!("  {label}: {glyphs}\n"));
}

// --- PNG + base64 -------------------------------------------------------------

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buffer, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("png header: {error}"))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| format!("png data: {error}"))?;
        writer
            .finish()
            .map_err(|error| format!("png finish: {error}"))?;
    }
    Ok(buffer)
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::inspect::{
        BitColumn, BitRow, FlagName, PairCell, PairMatrix, Register, RegisterGroup, Row, Sweep,
        SweepZone, Tone,
    };

    #[test]
    fn step_tick_tool_gated_on_tick_name() {
        // The default `SystemDebugger::tick_name` (a core with no tick finer
        // than an instruction) declines the tool.
        assert!(step_tick_tool(None).is_none());
        // A core that names a tick advertises it, naming the unit.
        let tool = step_tick_tool(Some("dot")).expect("a named tick advertises step_tick");
        assert_eq!(tool.name, "step_tick");
        assert!(tool.description.contains("dot"));
    }

    #[test]
    fn display_line_names_panel_and_standard() {
        use missingno_core::video::{DisplayTechnology, LcdPanel};

        assert_eq!(
            display_line(&DisplayTechnology::Lcd {
                native: (160, 144),
                panel: LcdPanel::PassiveStn,
                pixel_aspect: 1.0,
            }),
            "LCD (passive STN) 160x144"
        );
        assert_eq!(
            display_line(&DisplayTechnology::Lcd {
                native: (160, 144),
                panel: LcdPanel::ActiveTft,
                pixel_aspect: 1.0,
            }),
            "LCD (active TFT) 160x144"
        );
        assert_eq!(
            display_line(&DisplayTechnology::Crt {
                standard: missingno_core::TvStandard::Ntsc,
                pixel_aspect: 12.0 / 7.0,
            }),
            "CRT NTSC"
        );
        assert_eq!(
            display_line(&DisplayTechnology::Crt {
                standard: missingno_core::TvStandard::Secam,
                pixel_aspect: 12.0 / 7.0,
            }),
            "CRT SECAM"
        );
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn hex_dump_rows_and_ascii() {
        let dump = hex_dump(0xc000, b"AB\x00\xff");
        assert!(dump.starts_with("c000  41 42 00 ff"));
        assert!(dump.contains("AB.."));
    }

    #[test]
    fn sparkline_spans_bars_and_keeps_peaks() {
        // A 0..=15 ramp over a 4-bit channel walks the whole glyph range.
        let ramp: Vec<u8> = (0..=15).collect();
        let line = sparkline(&ramp, 15, SPARK_WIDTH);
        assert_eq!(line.chars().count(), 16);
        assert!(line.starts_with('▁'));
        assert!(line.ends_with('█'));

        // A lone spike downsampled hard still shows a full bar.
        let mut spiky = vec![0u8; 200];
        spiky[100] = 15;
        assert!(sparkline(&spiky, 15, SPARK_WIDTH).contains('█'));

        // Empty and zero-width windows render nothing.
        assert!(sparkline(&[], 15, SPARK_WIDTH).is_empty());
        assert!(sparkline(&[1, 2, 3], 15, 0).is_empty());
    }

    #[test]
    fn flags_register_renders_each_bit() {
        static FLAGS: [FlagName; 2] = [
            FlagName {
                name: "Z",
                bit: 7,
                help: None,
            },
            FlagName {
                name: "C",
                bit: 4,
                help: None,
            },
        ];
        let register = Register {
            name: "f",
            value: 0b1000_0000,
            bits: 8,
            style: ValueStyle::Flags(&FLAGS),
            help: None,
        };
        assert_eq!(render_register(&register), "[Z+ C-]");
    }

    fn synthetic_sections() -> Vec<Section> {
        static COLS: [FlagName; 0] = [];
        let _ = COLS;
        vec![Section {
            name: "CPU",
            summary: "pc 0100".into(),
            active: Some(true),
            detail: None,
            blocks: vec![
                SectionBlock::Registers(RegisterGroup {
                    name: "regs",
                    registers: vec![Register {
                        name: "a",
                        value: 0x12,
                        bits: 8,
                        style: ValueStyle::Hex,
                        help: None,
                    }],
                }),
                SectionBlock::Rows(vec![Row::flag("lcd on", true)]),
                SectionBlock::Sweeps(vec![Sweep::new("ly", 100, 154).zones(vec![
                    SweepZone {
                        name: "visible",
                        end: 144,
                        tone: Tone::Rendering,
                    },
                    SweepZone {
                        name: "vblank",
                        end: 154,
                        tone: Tone::Active,
                    },
                ])]),
                SectionBlock::Pixels(vec![PixelStrip::Bits {
                    label: "fifo",
                    cells: vec![true, false, true],
                    help: None,
                }]),
                SectionBlock::Table(BitTable {
                    columns: vec![BitColumn::plain("v")],
                    corner: None,
                    rows: vec![BitRow {
                        name: "ie",
                        bits: vec![true],
                        tone: Tone::Neutral,
                    }],
                }),
                SectionBlock::Relations(PairMatrix::new(
                    &["a", "b", "c"],
                    vec![
                        PairCell {
                            set: true,
                            help: None,
                        },
                        PairCell {
                            set: false,
                            help: None,
                        },
                        PairCell {
                            set: false,
                            help: None,
                        },
                    ],
                )),
            ],
        }]
    }

    #[test]
    fn describe_renders_every_block_kind() {
        let mut out = String::new();
        let sections = synthetic_sections();
        render_section_header(&sections[0], &mut out);
        for block in &sections[0].blocks {
            render_block(block, &mut out);
        }
        assert!(out.contains("== CPU =="));
        assert!(out.contains("pc 0100"));
        assert!(out.contains("a=12"));
        assert!(out.contains("● lcd on"));
        assert!(out.contains("ly 100/154 (visible)"));
        assert!(out.contains(&format!("fifo: {CELL_FILLED}{CELL_EMPTY}{CELL_FILLED}")));
        assert!(out.contains("ie"));
        // The pair matrix: labels on the diagonal, the set (a,b) pip on b's
        // row just before its label.
        assert!(out.contains("  a\n"));
        assert!(out.contains("● b"));
    }

    fn synthetic_atlas() -> TileAtlas {
        use missingno_core::graphics::AtlasRegion;
        // Tile 0's first row ramps 0,1,2,3 then mirrors; the rest are flat.
        let mut indices = vec![0u8; 4 * 64];
        indices[0..8].copy_from_slice(&[0, 1, 2, 3, 3, 2, 1, 0]);
        indices[2 * 64..3 * 64].fill(1);
        indices[3 * 64..4 * 64].fill(3);
        TileAtlas {
            label: "VRAM".into(),
            tile_width: 8,
            tile_height: 8,
            depth_bits: 2,
            indices,
            palettes: PaletteSet::FrontendShades,
            regions: vec![
                AtlasRegion {
                    label: "Block 0",
                    start: 0,
                    len: 2,
                    help: Some("$8000-$87FF"),
                },
                AtlasRegion {
                    label: "Block 1",
                    start: 2,
                    len: 2,
                    help: Some("$8800-$8FFF"),
                },
            ],
        }
    }

    #[test]
    fn atlas_survey_emits_png_and_summary() {
        let out = atlas_survey(&synthetic_atlas()).unwrap();
        match &out[0] {
            Content::Text(summary) => {
                assert!(summary.contains("VRAM"));
                assert!(summary.contains("4 tiles"));
                assert!(summary.contains("2bpp"));
            }
            _ => panic!("expected a summary line"),
        }
        match &out[1] {
            Content::Image { data, mime_type } => {
                assert_eq!(mime_type, "image/png");
                // Base64 of the 8-byte PNG signature (\x89PNG\r\n\x1a\n).
                assert!(data.starts_with("iVBORw0KGgo"));
            }
            _ => panic!("expected a PNG image"),
        }
    }

    #[test]
    fn atlas_survey_summary_lists_regions() {
        let summary = atlas_summary(&synthetic_atlas());
        assert!(summary.contains("regions:"));
        assert!(summary.contains("Block 0: tiles 0-1 ($8000-$87FF)"));
        assert!(summary.contains("Block 1: tiles 2-3 ($8800-$8FFF)"));
    }

    #[test]
    fn tile_detail_names_its_region() {
        let out = tile_detail(&synthetic_atlas(), 3).unwrap();
        let Content::Text(body) = &out[0] else {
            panic!("expected text");
        };
        assert!(body.starts_with("tile 3 of VRAM (Block 1):"));
    }

    #[test]
    fn atlas_pixels_has_plausible_dimensions() {
        // Four 8×8 tiles over 16 columns is one tile-row: 128×8.
        let (width, height, pixels) = atlas_pixels(&synthetic_atlas(), ATLAS_COLUMNS);
        assert_eq!((width, height), (128, 8));
        assert_eq!(pixels.len() as u32, width * height * 4);
    }

    #[test]
    fn tile_detail_shows_glyphs_and_indices() {
        let out = tile_detail(&synthetic_atlas(), 0).unwrap();
        let Content::Text(body) = &out[0] else {
            panic!("expected text");
        };
        // The 0,1,2,3 ramp maps to the four distinct shade glyphs.
        assert!(body.contains("·░▒▓▓▒░·"));
        assert!(body.contains("indices:"));
        assert!(body.contains("0 1 2 3 3 2 1 0"));
    }

    #[test]
    fn objects_table_and_object_detail() {
        let table = ObjectTable {
            label: "OAM".into(),
            atlas: 0,
            object_height: 16,
            objects: vec![Object {
                index: 0,
                x: -8,
                y: -16,
                tile: 2,
                on_screen: false,
                palette: Some(3),
                bank: None,
                flip_x: true,
                flip_y: false,
                priority: true,
            }],
        };
        let listing = objects_table(&table);
        assert!(listing.contains("OAM"));
        assert!(listing.contains("8×16"));
        assert!(listing.contains("-8"));
        assert!(listing.contains("beh")); // priority: behind background

        let graphics = GraphicsView {
            atlases: vec![synthetic_atlas()],
            maps: vec![],
            objects: Some(table.clone()),
        };
        let out = object_detail(&graphics, &table, 0).unwrap();
        let Content::Text(body) = &out[0] else {
            panic!("expected text");
        };
        assert!(body.contains("object 0"));
        assert!(body.contains("palette=3"));
        // 8×16 stacks tile 2 (the top &!1 slot, index-1 fill) over tile 3.
        assert!(body.contains("sprite (8×16)"));
        assert_eq!(body.matches('░').count(), 8 * 8); // tile 2 fills the top slot
        assert_eq!(body.matches('▓').count(), 8 * 8); // tile 3 fills the bottom slot
    }
}
