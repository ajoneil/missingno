//! Client of the app's UI-automation socket. The app publishes a
//! newline-delimited JSON-RPC surface on a Unix socket named by pid; this
//! connects, completes the `ui/info` handshake, and forwards `tools/list` and
//! `tools/call` frames verbatim.
//!
//! The runtime-directory and socket-client logic is duplicated from the
//! session's attach client rather than shared, so this binary links neither the
//! session nor the app.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// How long a discovery probe or handshake waits for a host to answer before
/// calling the socket stale.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// What a window says about itself at the `ui/info` handshake.
#[derive(Clone, Debug)]
pub struct UiInfo {
    pub path: PathBuf,
    pub app: String,
    pub pid: u32,
    pub version: String,
}

impl UiInfo {
    /// A one-line rendering for a listing an agent or a user reads.
    pub fn summary(&self) -> String {
        format!(
            "pid {} — {} v{} at {}",
            self.pid,
            self.app,
            self.version,
            self.path.display()
        )
    }
}

/// The per-user directory the app publishes into: the XDG runtime directory when
/// one is provided, else a uid-scoped directory under the system temp dir.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("missingno"),
        // SAFETY: `getuid` reads the calling process's own credentials.
        _ => std::env::temp_dir().join(format!("missingno-{}", unsafe { getuid() })),
    }
}

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Why attaching to a published window failed.
#[derive(Debug)]
pub enum AttachError {
    /// Nothing is listening on the socket — the file outlived its host.
    NoListener,
    Failed(String),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::NoListener => f.write_str("no window is listening there"),
            AttachError::Failed(reason) => f.write_str(reason),
        }
    }
}

impl From<AttachError> for String {
    fn from(error: AttachError) -> String {
        error.to_string()
    }
}

/// Every reachable window published in the default runtime directory.
pub fn discover() -> Vec<UiInfo> {
    discover_in(&runtime_dir())
}

/// Every reachable window published in `dir`. A `ui-*.sock` with nothing behind
/// it is unlinked rather than reported, so a crashed window neither haunts the
/// listing nor blocks the next one from taking its name.
pub fn discover_in(dir: &Path) -> Vec<UiInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut windows = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_ui_socket(&path) {
            continue;
        }
        match probe(&path) {
            Probe::Answered(info) => windows.push(info),
            // Something holds the socket but did not answer in time: a busy host
            // is still a live one. Only a refused connection proves it is gone.
            Probe::Silent => {}
            Probe::NoListener => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    windows.sort_by_key(|window| window.pid);
    windows
}

fn is_ui_socket(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("ui-") && name.ends_with(".sock"))
}

/// What asking a socket file who it is revealed about its host.
enum Probe {
    Answered(UiInfo),
    /// Connected, but the handshake did not complete — host present, busy or wedged.
    Silent,
    /// Nothing is listening: the file outlived its host.
    NoListener,
}

fn probe(path: &Path) -> Probe {
    match UiClient::connect(path) {
        Ok(client) => Probe::Answered(client.info().clone()),
        Err(AttachError::NoListener) => Probe::NoListener,
        Err(AttachError::Failed(_)) => Probe::Silent,
    }
}

/// Why a request produced no result.
#[derive(Debug)]
pub enum RequestError {
    /// The connection failed — the window is unreachable.
    Transport(String),
    /// The window answered with an error frame; the connection is fine.
    Answered(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::Transport(message) | RequestError::Answered(message) => {
                f.write_str(message)
            }
        }
    }
}

/// A connection to a published window. Requests are strictly request/response on
/// one connection, so ids are assigned locally and answers arrive in order.
pub struct UiClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    info: UiInfo,
}

impl UiClient {
    /// Connect to the window published at `path` and complete the handshake.
    pub fn connect(path: &Path) -> Result<Self, AttachError> {
        let stream = UnixStream::connect(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
                AttachError::NoListener
            }
            _ => AttachError::Failed(error.to_string()),
        })?;
        stream
            .set_read_timeout(Some(PROBE_TIMEOUT))
            .map_err(|error| AttachError::Failed(error.to_string()))?;
        let writer = stream
            .try_clone()
            .map_err(|error| AttachError::Failed(error.to_string()))?;
        let mut client = UiClient {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
            info: UiInfo {
                path: path.to_path_buf(),
                app: String::new(),
                pid: 0,
                version: String::new(),
            },
        };
        let info = client
            .request("ui/info", json!({}))
            .map_err(|error| AttachError::Failed(error.to_string()))?;
        client.info = UiInfo {
            path: path.to_path_buf(),
            app: field(&info, "app"),
            pid: info.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
            version: field(&info, "version"),
        };
        // Past the handshake a call may run for as long as the tool takes.
        let _ = client.writer.set_read_timeout(None);
        let _ = client.reader.get_ref().set_read_timeout(None);
        Ok(client)
    }

    /// Who is on the other end.
    pub fn info(&self) -> &UiInfo {
        &self.info
    }

    /// Send one request and read its answer's `result`.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, RequestError> {
        let transport = |error: std::io::Error| RequestError::Transport(error.to_string());
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.writer, "{frame}").map_err(transport)?;
        self.writer.flush().map_err(transport)?;

        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => {
                return Err(RequestError::Transport(
                    "the window closed the connection".into(),
                ));
            }
            Ok(_) => {}
            Err(error) => return Err(transport(error)),
        }
        let response: Value = serde_json::from_str(&line)
            .map_err(|error| RequestError::Transport(error.to_string()))?;
        if let Some(error) = response.get("error") {
            return Err(RequestError::Answered(field(error, "message")));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| RequestError::Transport("the window answered without a result".into()))
    }
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A stub UI host: answer one client's `ui/info` on `listener`, then stop.
    fn spawn_stub(listener: UnixListener, pid: u32) {
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let mut writer = stream.try_clone().expect("clone");
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let id = serde_json::from_str::<Value>(&line)
                        .ok()
                        .and_then(|frame| frame.get("id").cloned())
                        .unwrap_or(Value::Null);
                    let response = json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "app": "net.andyofniall.missingno",
                            "pid": pid,
                            "version": "9.9.9",
                        },
                    });
                    let _ = writeln!(writer, "{response}");
                    let _ = writer.flush();
                }
            }
        });
    }

    #[test]
    fn discovery_lists_live_windows_and_prunes_dead_sockets() {
        let dir =
            std::env::temp_dir().join(format!("missingno-remote-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        // A live window: a bound listener with a stub host answering ui/info.
        let live_path = dir.join("ui-4242.sock");
        let listener = UnixListener::bind(&live_path).expect("bind live");
        spawn_stub(listener, 4242);

        // A dead window: a bound-then-dropped listener leaves the file behind
        // with nothing answering, so a connect there is refused.
        let dead_path = dir.join("ui-1.sock");
        drop(UnixListener::bind(&dead_path).expect("bind dead"));

        // A file that is not a UI socket must be ignored, not pruned.
        let other_path = dir.join("session-5.sock");
        std::fs::write(&other_path, b"not a socket").expect("write other");

        let windows = discover_in(&dir);

        assert_eq!(windows.len(), 1, "only the live window: {windows:?}");
        assert_eq!(windows[0].pid, 4242);
        assert_eq!(windows[0].app, "net.andyofniall.missingno");
        assert_eq!(windows[0].version, "9.9.9");

        assert!(!dead_path.exists(), "the dead socket is pruned");
        assert!(other_path.exists(), "a non-ui file is left alone");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
