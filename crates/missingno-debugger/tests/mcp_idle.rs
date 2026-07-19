//! With the `mcp` feature, drive the idle MCP server over stdio end to end:
//! a static server with no ROM advertises three tools, gains the full set once
//! `load_rom` recognises a Game Boy ROM through the factory, and returns to the
//! three-tool idle set on `eject`.

#![cfg(all(feature = "mcp", feature = "gb"))]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Write a minimal 32 KiB all-NOP `.gb` ROM to a temp path; the `.gb` extension
/// makes the registry claim it and the DMG core boots to PC 0x0100.
fn write_minimal_rom() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("missingno-mcp-idle-{}.gb", std::process::id()));
    std::fs::write(&path, vec![0x00u8; 0x8000]).expect("write temp ROM");
    path
}

/// Run the idle MCP server over the given request lines, returning the response
/// object for each request id, in the order the server emitted them.
fn exchange(lines: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_missingno-debugger"))
        .arg("--mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn missingno-debugger --mcp");

    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        for line in lines {
            writeln!(stdin, "{}", serde_json::to_string(line).unwrap()).unwrap();
        }
        // Dropping stdin at the end of this scope sends EOF, ending the loop.
    }
    child.stdin.take();

    let stdout = child.stdout.take().expect("child stdout");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.expect("stdout line")).expect("valid JSON-RPC"))
        .collect();
    child.wait().expect("child exits");
    responses
}

/// The names of the tools in a `tools/list` result.
fn tool_names(result: &Value) -> Vec<String> {
    result["result"]["tools"]
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// A tool result's first text content.
fn text_of(result: &Value) -> String {
    result["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn idle_server_loads_a_rom_and_ejects() {
    let rom = write_minimal_rom();
    let rom = rom.to_str().unwrap();

    let responses = exchange(&[
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "load_rom", "arguments": { "path": rom } } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": { "name": "step_tick", "arguments": { "count": 4 } } }),
        json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                "params": { "name": "describe_machine" } }),
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": "eject" } }),
        json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 9, "method": "shutdown" }),
    ]);

    // One response per request, in id order.
    assert_eq!(responses.len(), 9);

    // initialize: the server names itself idle before any ROM is loaded.
    let name = responses[0]["result"]["serverInfo"]["name"]
        .as_str()
        .unwrap();
    assert!(name.contains("idle"), "idle handshake, got {name}");

    // Idle advertises exactly load_rom / eject / status.
    let idle = tool_names(&responses[1]);
    assert_eq!(idle.len(), 3, "idle tools: {idle:?}");
    for expected in ["load_rom", "eject", "status"] {
        assert!(
            idle.contains(&expected.to_string()),
            "idle missing {expected}"
        );
    }

    // load_rom recognises the ROM as a Game Boy and succeeds.
    assert_eq!(responses[2]["result"]["isError"], json!(false));
    assert!(text_of(&responses[2]).contains("Game Boy"));

    // The full tool set now appears (well past three), still offering eject.
    let full = tool_names(&responses[3]);
    assert!(full.len() > 3, "full tools: {full:?}");
    for expected in ["step_tick", "describe_machine", "read_memory", "eject"] {
        assert!(
            full.contains(&expected.to_string()),
            "full missing {expected}"
        );
    }

    // The loaded tools drive the console: four dots complete one NOP.
    assert_eq!(responses[4]["result"]["isError"], json!(false));
    assert!(text_of(&responses[4]).contains("dot"));
    assert_eq!(responses[5]["result"]["isError"], json!(false));
    assert!(text_of(&responses[5]).contains("CPU"));

    // eject returns to the three-tool idle set.
    assert_eq!(responses[6]["result"]["isError"], json!(false));
    let after = tool_names(&responses[7]);
    assert_eq!(after.len(), 3, "post-eject tools: {after:?}");

    let _ = std::fs::remove_file(rom);
}
