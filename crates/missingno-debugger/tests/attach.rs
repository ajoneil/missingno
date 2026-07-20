//! Cross-process attach: a published session answers the same tool vocabulary
//! over its socket that a local client gets in-process, discovery reports only
//! sessions that actually answer, and the session-level tools drive the machine
//! through its command queue.

#![cfg(all(unix, feature = "mcp", feature = "gb"))]

use std::path::{Path, PathBuf};

use missingno_debugger::attach::{AttachClient, AttachEndpoint, Publication, discover_in};
use missingno_debugger::mcp;
use missingno_debugger::{SessionHandle, SharedSession};
use serde_json::{Value, json};

/// A 32 KiB all-NOP `.gb` ROM: the extension makes the registry claim it, and
/// the DMG core boots to PC 0x0100.
fn gb_session() -> SharedSession {
    let rom = vec![0x00u8; 0x8000];
    let console = missingno_debugger::factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory claims a .gb ROM");
    SharedSession::spawn(console.into_debugger().ok().expect("gb has a debugger"))
}

/// The same ROM hosted as a plain console, with no debugger surface.
fn gb_console_session() -> SharedSession {
    let rom = vec![0x00u8; 0x8000];
    let console = missingno_debugger::factory::create_console(Path::new("test.gb"), &rom)
        .expect("factory should not error")
        .expect("gb factory claims a .gb ROM");
    SharedSession::spawn_console(console)
}

/// A fresh directory for one test to publish into, so per-pid socket names
/// cannot collide between tests in this process.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("missingno-attach-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn publication() -> Publication {
    Publication {
        title: "TESTROM".into(),
        core: "Game Boy".into(),
    }
}

fn call(client: &mut AttachClient, name: &str, arguments: Value) -> Value {
    client
        .request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .expect("the session answers")
}

fn text_of(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn tool_names(result: &Value) -> Vec<String> {
    result["tools"]
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_client_attaches_over_the_socket_and_drives_the_session() {
    let dir = scratch_dir("round-trip");
    let session = gb_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("publish the session");

    let mut client = AttachClient::connect(endpoint.path()).expect("attach");
    assert_eq!(client.info().title, "TESTROM");
    assert_eq!(client.info().core, "Game Boy");
    assert_eq!(client.info().pid, std::process::id());
    assert!(client.info().debugger);

    // The published surface carries both the session-level tools and the
    // debugger's own.
    let names = tool_names(&client.request("tools/list", json!({})).expect("list"));
    for expected in [
        "run",
        "pause",
        "set_control",
        "status",
        "read_registers",
        "step",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }

    // A command drives the real machine: four dots complete one NOP.
    let before = text_of(&call(&mut client, "status", json!({})));
    assert!(before.contains("pc: 0100"), "status was: {before}");
    let stepped = call(&mut client, "step_tick", json!({ "count": 4 }));
    assert_eq!(stepped["isError"], json!(false));
    let after = text_of(&call(&mut client, "status", json!({})));
    assert!(after.contains("pc: 0101"), "status was: {after}");

    // A readout reaches the same console the command moved.
    let registers = call(&mut client, "read_registers", json!({}));
    assert_eq!(registers["isError"], json!(false));

    drop(client);
    drop(endpoint);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_the_endpoint_removes_the_socket() {
    let dir = scratch_dir("cleanup");
    let session = gb_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("publish the session");
    let path = endpoint.path().to_path_buf();
    assert!(path.exists());

    drop(endpoint);
    assert!(!path.exists(), "the socket file outlived the session");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discovery_reports_live_sessions_and_clears_stale_ones() {
    let dir = scratch_dir("discovery");

    // A socket file whose host has gone: nothing answers on it.
    let stale = dir.join("session-4294967290.sock");
    std::fs::write(&stale, b"").expect("write a stale socket file");
    assert!(
        discover_in(&dir).is_empty(),
        "a dead socket is not a session"
    );
    assert!(!stale.exists(), "a dead socket file should be cleared");

    let session = gb_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("publish the session");

    // A second stale file alongside the live one is dropped from the listing.
    std::fs::write(&stale, b"").expect("write a stale socket file");
    let found = discover_in(&dir);
    assert_eq!(found.len(), 1, "found: {found:?}");
    assert_eq!(found[0].pid, std::process::id());
    assert_eq!(found[0].title, "TESTROM");
    assert!(!stale.exists());

    drop(endpoint);
    assert!(
        discover_in(&dir).is_empty(),
        "a closed session is no longer reachable"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_socket_file_does_not_block_publishing() {
    let dir = scratch_dir("stale-bind");
    // A file exactly where this process would publish, left by a dead host.
    let path = dir.join(format!("session-{}.sock", std::process::id()));
    std::fs::write(&path, b"").expect("write a stale socket file");

    let session = gb_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("the stale file is replaced, not treated as a live session");
    assert!(AttachClient::connect(endpoint.path()).is_ok());

    drop(endpoint);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_client_attaches_alongside_the_first() {
    let dir = scratch_dir("two-clients");
    let session = gb_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("publish the session");

    let mut first = AttachClient::connect(endpoint.path()).expect("first attaches");
    let mut second = AttachClient::connect(endpoint.path()).expect("second attaches");

    // One client stepping is visible to the other: the machine is shared.
    call(&mut first, "step_tick", json!({ "count": 4 }));
    assert!(text_of(&call(&mut second, "status", json!({}))).contains("pc: 0101"));

    // A disconnect leaves the session and the other client working.
    drop(first);
    call(&mut second, "step_tick", json!({ "count": 4 }));
    assert!(text_of(&call(&mut second, "status", json!({}))).contains("pc: 0102"));

    drop(second);
    drop(endpoint);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_console_session_publishes_only_the_tools_it_can_answer() {
    let dir = scratch_dir("console");
    let session = gb_console_session();
    let endpoint = AttachEndpoint::open_in(&dir, session.handle(), publication())
        .expect("publish the session");

    let mut client = AttachClient::connect(endpoint.path()).expect("attach");
    assert!(!client.info().debugger);

    let names = tool_names(&client.request("tools/list", json!({})).expect("list"));
    assert!(names.contains(&"run".to_string()));
    assert!(
        !names.contains(&"read_registers".to_string()),
        "a plain console advertises no debugger readouts: {names:?}"
    );

    // Asking anyway is refused rather than dropped unanswered.
    let refused = call(&mut client, "read_registers", json!({}));
    assert_eq!(refused["isError"], json!(true));

    drop(client);
    drop(endpoint);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- the session-level tools, in process --------------------------------------

fn tool(handle: &SessionHandle, name: &str, args: Value) -> Result<String, String> {
    let outcome = mcp::call_session_tool(handle, "Game Boy", name, &args)
        .unwrap_or_else(|| Err(format!("unknown tool: {name}")))?;
    Ok(match outcome.first() {
        Some(mcp::Content::Text(body)) => body.clone(),
        _ => String::new(),
    })
}

#[test]
fn run_and_pause_move_the_session_between_states() {
    let session = gb_session();
    let handle = session.handle();
    assert!(!handle.is_running());

    // Both block until the loop has reached the state, so what they report is
    // settled rather than merely requested.
    assert!(
        tool(&handle, "run", json!({}))
            .unwrap()
            .contains("running: true")
    );
    assert!(handle.is_running());

    assert!(
        tool(&handle, "pause", json!({}))
            .unwrap()
            .contains("running: false")
    );
    assert!(!handle.is_running());
}

#[test]
fn recording_captures_agent_driven_input() {
    let dir = scratch_dir("recording");
    let path = dir.join("agent.mprc");
    let session = gb_session();
    let handle = session.handle();

    tool(&handle, "start_recording", json!({ "path": path })).expect("start recording");
    assert!(handle.is_recording());
    assert!(
        tool(&handle, "status", json!({}))
            .unwrap()
            .contains("recording: true")
    );

    // The tool routes through the session's command queue, so the input is
    // captured rather than applied behind the recorder's back.
    tool(
        &handle,
        "set_control",
        json!({ "control": 0, "pressed": true }),
    )
    .expect("press");
    tool(&handle, "run", json!({})).expect("run");
    std::thread::sleep(std::time::Duration::from_millis(150));
    tool(&handle, "pause", json!({})).expect("pause");
    tool(
        &handle,
        "set_control",
        json!({ "control": 0, "pressed": false }),
    )
    .expect("release");

    tool(&handle, "stop_recording", json!({})).expect("stop recording");
    assert!(!handle.is_recording());

    let bytes = std::fs::read(&path).expect("the recording was written");
    // Readable at all is half the assertion: the release lands after the last
    // stepped frame, and a timeline that kept it would not decode.
    let recording =
        missingno_core::recording::Recording::from_bytes(&bytes).expect("a readable recording");
    assert!(
        recording.inputs.iter().any(|input| input.control.0 == 0),
        "the agent's press was captured: {:?}",
        recording.inputs
    );
    assert!(
        recording
            .inputs
            .iter()
            .all(|input| input.frame < recording.frames),
        "every captured input lands on a frame replay steps: {:?}",
        recording.inputs
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replaying_is_refused_while_recording() {
    let dir = scratch_dir("replay-guard");
    let path = dir.join("guarded.mprc");
    let session = gb_session();
    let handle = session.handle();

    tool(&handle, "start_recording", json!({ "path": path.clone() })).expect("start recording");
    let refused = tool(&handle, "play_recording", json!({ "path": path }));
    assert!(refused.is_err(), "a replay during capture must be refused");

    let _ = std::fs::remove_dir_all(&dir);
}
