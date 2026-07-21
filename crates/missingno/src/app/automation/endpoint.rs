//! The UI-automation socket host: this process publishes its own frontend on a
//! Unix socket, so an external agent can enumerate and drive the window. It
//! speaks the same newline-delimited JSON-RPC the session attach socket does;
//! the one handshake method is `ui/info`.
//!
//! Modeled on the session's attach endpoint. The socket lives in the shared
//! runtime directory, named by pid, created mode 0600 under a 0700 directory,
//! and removed on drop. It is app-lifetime — open whenever the setting or flag
//! is on, whether or not a game is loaded.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use super::bridge::{AutomationCall, SharedSink};

/// How long an accept loop waits between polls, bounding shutdown latency.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// How long a socket client waits for the UI thread to answer a call.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

fn socket_name(pid: u32) -> String {
    format!("ui-{pid}.sock")
}

/// A published automation surface: the listening socket plus its accept thread.
/// Dropping it stops accepting and removes the socket file.
pub struct AutomationEndpoint {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AutomationEndpoint {
    /// Publish this process's automation surface in the default runtime
    /// directory, reading calls from `sink`.
    pub fn open(sink: SharedSink) -> std::io::Result<Self> {
        Self::open_in(&missingno_session::attach::runtime_dir(), sink)
    }

    /// Publish in `dir`, creating it user-only if absent. A socket file left by
    /// a dead host of the same name is replaced; one whose host still answers
    /// is an error rather than a silent takeover.
    pub fn open_in(dir: &Path, sink: SharedSink) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;

        let path = dir.join(socket_name(std::process::id()));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("an automation host already answers on {}", path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("ui-automation-endpoint".into())
            .spawn({
                let stop = stop.clone();
                move || accept_loop(listener, sink, stop)
            })?;

        Ok(AutomationEndpoint {
            path,
            stop,
            thread: Some(thread),
        })
    }

    /// Where this surface is published.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AutomationEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop(listener: UnixListener, sink: SharedSink, stop: Arc<AtomicBool>) {
    let mut clients: Vec<JoinHandle<()>> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let sink = sink.clone();
                let stop = stop.clone();
                if let Ok(thread) = std::thread::Builder::new()
                    .name("ui-automation-client".into())
                    .spawn(move || serve_client(stream, sink, stop))
                {
                    clients.push(thread);
                }
                clients.retain(|client| !client.is_finished());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL)
            }
            Err(_) => break,
        }
    }
    for client in clients {
        let _ = client.join();
    }
}

fn serve_client(stream: UnixStream, sink: SharedSink, stop: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(ACCEPT_POLL));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return,
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = answer(&line, &sink);
        let encoded = serde_json::to_string(&response).unwrap_or_default();
        if writeln!(writer, "{encoded}").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

/// Dispatch one request frame.
fn answer(line: &str, sink: &SharedSink) -> Value {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => return error_frame(Value::Null, &format!("parse error: {error}")),
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
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

fn success_frame(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_frame(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

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

        let endpoint = AutomationEndpoint::open_in(&dir, shared).expect("open endpoint");
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
