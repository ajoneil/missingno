//! The UI-automation socket host: this process publishes its own UI on a
//! Unix socket, so an external agent can enumerate and drive the window. It
//! speaks the same newline-delimited JSON-RPC the session attach socket does;
//! the one handshake method is `ui/info`.
//!
//! The socket is the session crate's shared host: it lives in the same runtime
//! directory, named by pid, created mode 0600 under a 0700 directory, and
//! removed on drop. It is app-lifetime — open whenever the setting or flag is
//! on, whether or not a game is loaded.

use std::path::Path;
use std::time::Duration;

use missingno_session::attach::{
    HostSpec, PartialFrames, Request, SocketHost, error_frame, success_frame,
};
use serde_json::{Value, json};

use super::bridge::{AutomationCall, SharedSink};

/// How long a socket client waits for the UI thread to answer a call.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

const AUTOMATION_HOST: HostSpec = HostSpec {
    socket_prefix: "ui",
    occupant: "an automation host",
    accept_thread: "ui-automation-endpoint",
    client_thread: "ui-automation-client",
    partial_frames: PartialFrames::Dropped,
};

/// Publish this process's automation surface in the default runtime directory,
/// reading calls from `sink`.
pub fn open(sink: SharedSink) -> std::io::Result<SocketHost> {
    open_in(&missingno_session::attach::runtime_dir(), sink)
}

/// Publish in `dir`.
pub fn open_in(dir: &Path, sink: SharedSink) -> std::io::Result<SocketHost> {
    SocketHost::open_in(dir, AUTOMATION_HOST, move |line, _| {
        Some(answer(line, &sink))
    })
}

/// Dispatch one request frame.
fn answer(line: &str, sink: &SharedSink) -> Value {
    let Request { id, method, params } = match Request::parse(line) {
        Ok(request) => request,
        Err(error) => return error_frame(Value::Null, &format!("parse error: {error}")),
    };

    match method.as_str() {
        "ui/info" => success_frame(
            id,
            json!({
                "app": "net.andyofniall.missingno",
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        "tools/list" => success_frame(id, super::tools::tools_json()),
        "tools/call" => match call(sink, &params) {
            Ok(result) => success_frame(id, result),
            Err(reason) => error_frame(id, &reason),
        },
        other => error_frame(id, &format!("method not found: {other}")),
    }
}

/// Forward a `tools/call` to the UI thread and wait for its answer.
fn call(sink: &SharedSink, params: &Value) -> Result<Value, String> {
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("'name' (string) is required")?
        .to_string();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let sink = sink
        .get()
        .ok_or("the automation surface is not ready yet")?;

    let (reply, answer) = std::sync::mpsc::channel();
    sink.unbounded_send(AutomationCall { tool, args, reply })
        .map_err(|_| "the window is not accepting calls".to_string())?;
    answer
        .recv_timeout(REPLY_TIMEOUT)
        .map_err(|_| "the window did not answer in time".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    /// Open an endpoint in a temp dir with a stub UI thread that answers every
    /// call, connect a raw client, and exchange the three methods.
    #[test]
    fn framing_round_trip() {
        let dir = std::env::temp_dir().join(format!("missingno-ui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let shared = SharedSink::default();

        // A stub executor: hand it the sink, answer each call with a canned
        // result. Stands in for the app's update loop.
        let (sink, mut calls) = iced::futures::channel::mpsc::unbounded::<AutomationCall>();
        shared.set(sink);
        let stub = std::thread::spawn(move || {
            use iced::futures::StreamExt;
            while let Some(call) = iced::futures::executor::block_on(calls.next()) {
                let _ = call.reply.send(json!({
                    "content": [ { "type": "text", "text": format!("ran {}", call.tool) } ],
                    "isError": false,
                }));
            }
        });

        let endpoint = open_in(&dir, shared).expect("open endpoint");
        let path = endpoint.path().to_path_buf();

        let stream = UnixStream::connect(&path).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        let request =
            |writer: &mut UnixStream, reader: &mut BufReader<UnixStream>, frame: Value| {
                writeln!(writer, "{frame}").unwrap();
                writer.flush().unwrap();
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                serde_json::from_str::<Value>(&line).unwrap()
            };

        let info = request(
            &mut writer,
            &mut reader,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "ui/info" }),
        );
        assert_eq!(info["result"]["app"], "net.andyofniall.missingno");

        let list = request(
            &mut writer,
            &mut reader,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        );
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"status"));
        assert!(names.contains(&"ui_tree"));

        let called = request(
            &mut writer,
            &mut reader,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "status", "arguments": {} } }),
        );
        assert_eq!(called["result"]["isError"], false);
        assert_eq!(called["result"]["content"][0]["text"], "ran status");

        drop(endpoint);
        drop(writer);
        drop(reader);
        stub.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
