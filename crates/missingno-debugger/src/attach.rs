//! Cross-process attach: a session host publishes the session it owns on a Unix
//! domain socket, and an external client drives it there instead of creating a
//! console of its own.
//!
//! The socket speaks the same vocabulary the MCP transport already does —
//! newline-delimited JSON-RPC frames carrying `tools/list` and `tools/call` —
//! so an attached client forwards a frame verbatim and an attached tool call is
//! indistinguishable from a local one. One extra method, `session/info`,
//! answers the handshake a discovering client needs.
//!
//! Sockets live one-per-session in the user's runtime directory, named by the
//! host process id. Reach is filesystem reach: the directory is user-scoped and
//! each socket is created mode 0600, and a host publishes only when its
//! frontend asks it to.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use crate::mcp;
use crate::shared::SessionHandle;

/// How long an accept loop waits between polls, bounding how long a shutdown
/// takes to be noticed.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// How long a discovery probe waits for a host to answer before calling the
/// socket stale.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// What a host says about itself, so a discovering client can tell one running
/// session from another before attaching.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub path: PathBuf,
    pub pid: u32,
    pub title: String,
    pub core: String,
    /// Whether the session hosts a debugger; a plain-console session serves only
    /// the run/control/state tools.
    pub debugger: bool,
}

impl SessionInfo {
    /// A one-line rendering for a listing an agent or a user reads.
    pub fn summary(&self) -> String {
        let kind = if self.debugger {
            "debugger"
        } else {
            "emulator"
        };
        format!(
            "pid {} — {} ({}, {kind}) at {}",
            self.pid,
            self.title,
            self.core,
            self.path.display()
        )
    }
}

/// What a host publishes about itself, supplied by the frontend rather than read
/// from the session: a plain-console session has no debugger surface to ask.
#[derive(Clone, Debug)]
pub struct Publication {
    pub title: String,
    pub core: String,
}

/// The per-user directory sessions publish into: the XDG runtime directory when
/// the session manager provides one, else a uid-scoped directory under the
/// system temp dir.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("missingno"),
        // SAFETY: `getuid` reads the calling process's own credentials.
        _ => std::env::temp_dir().join(format!("missingno-{}", unsafe { libc::getuid() })),
    }
}

fn socket_name(pid: u32) -> String {
    format!("session-{pid}.sock")
}

/// Every reachable session published in the default runtime directory.
pub fn discover() -> Vec<SessionInfo> {
    discover_in(&runtime_dir())
}

/// Every reachable session published in `dir`. A socket file with nothing behind
/// it is unlinked rather than reported — a crashed session must not haunt the
/// listing, nor block the next one from taking its name.
pub fn discover_in(dir: &Path) -> Vec<SessionInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "sock") {
            continue;
        }
        match probe(&path) {
            Probe::Answered(info) => sessions.push(info),
            // Something holds the socket but did not answer in time: a busy host
            // is still a live one, so leave its file alone and let the next scan
            // find it. Only a refused connection proves the host is gone.
            Probe::Silent => {}
            Probe::NoListener => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    sessions.sort_by_key(|session| session.pid);
    sessions
}

/// Why attaching to a published session failed.
#[derive(Debug)]
pub enum AttachError {
    /// Nothing is listening on the socket — the file outlived its host, so it is
    /// safe to clear away.
    NoListener,
    Failed(String),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::NoListener => f.write_str("no session is listening there"),
            AttachError::Failed(reason) => f.write_str(reason),
        }
    }
}

impl From<AttachError> for String {
    fn from(error: AttachError) -> String {
        error.to_string()
    }
}

/// What asking a socket file who it is revealed about its host.
enum Probe {
    Answered(SessionInfo),
    /// Connected, but the handshake did not complete — host present, busy or wedged.
    Silent,
    /// Nothing is listening: the file outlived its host.
    NoListener,
}

/// Ask the socket at `path` who it is.
fn probe(path: &Path) -> Probe {
    match AttachClient::connect(path) {
        Ok(client) => Probe::Answered(client.info().clone()),
        Err(AttachError::NoListener) => Probe::NoListener,
        Err(AttachError::Failed(_)) => Probe::Silent,
    }
}

// --- host ---------------------------------------------------------------------

/// A published session: the listening socket plus the thread accepting clients.
/// Dropping it stops accepting and removes the socket file, so a session that
/// ends leaves nothing behind for a discovering client to trip over.
pub struct AttachEndpoint {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AttachEndpoint {
    /// Publish `handle` in the default runtime directory.
    pub fn open(handle: SessionHandle, publication: Publication) -> std::io::Result<Self> {
        Self::open_in(&runtime_dir(), handle, publication)
    }

    /// Publish `handle` in `dir`, creating the directory user-only if it does not
    /// exist. A socket file left by a dead host of the same name is replaced;
    /// one whose host still answers is an error rather than a silent takeover.
    pub fn open_in(
        dir: &Path,
        handle: SessionHandle,
        publication: Publication,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;

        let path = dir.join(socket_name(std::process::id()));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("a session already answers on {}", path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("attach-endpoint".into())
            .spawn({
                let stop = stop.clone();
                move || accept_loop(listener, handle, publication, stop)
            })?;

        Ok(AttachEndpoint {
            path,
            stop,
            thread: Some(thread),
        })
    }

    /// Where this session is published, for a host that wants to show the user.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AttachEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop(
    listener: UnixListener,
    handle: SessionHandle,
    publication: Publication,
    stop: Arc<AtomicBool>,
) {
    let mut clients: Vec<JoinHandle<()>> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let handle = handle.clone();
                let publication = publication.clone();
                let stop = stop.clone();
                if let Ok(thread) = std::thread::Builder::new()
                    .name("attach-client".into())
                    .spawn(move || serve_client(stream, handle, publication, stop))
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

/// Answer one client's frames until it disconnects or the endpoint closes. A
/// disconnect ends this thread only — the session and its other clients are
/// untouched.
fn serve_client(
    stream: UnixStream,
    handle: SessionHandle,
    publication: Publication,
    stop: Arc<AtomicBool>,
) {
    // A read timeout is what lets a quiet client notice the endpoint closing.
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
        let response = answer(&line, &handle, &publication);
        let encoded = serde_json::to_string(&response).unwrap_or_default();
        if writeln!(writer, "{encoded}").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

/// Dispatch one request frame against the hosted session.
fn answer(line: &str, handle: &SessionHandle, publication: &Publication) -> Value {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => return error_frame(Value::Null, &format!("parse error: {error}")),
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "session/info" => success_frame(
            id,
            json!({
                "pid": std::process::id(),
                "title": publication.title,
                "core": publication.core,
                "debugger": handle.is_debugger(),
            }),
        ),
        "tools/list" => success_frame(id, mcp::session_tools_json(handle)),
        "tools/call" => success_frame(
            id,
            mcp::call_session_tool_json(handle, &publication.core, &params),
        ),
        other => error_frame(id, &format!("method not found: {other}")),
    }
}

fn success_frame(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_frame(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

// --- client -------------------------------------------------------------------

/// A connection to a published session. Requests are strictly request/response
/// on one connection, so ids are assigned locally and answers arrive in order.
pub struct AttachClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    info: SessionInfo,
}

impl AttachClient {
    /// Connect to the session published at `path` and complete the handshake.
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
        let mut client = AttachClient {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
            info: SessionInfo {
                path: path.to_path_buf(),
                pid: 0,
                title: String::new(),
                core: String::new(),
                debugger: false,
            },
        };
        let info = client
            .request("session/info", json!({}))
            .map_err(AttachError::Failed)?;
        client.info = SessionInfo {
            path: path.to_path_buf(),
            pid: info.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
            title: field(&info, "title"),
            core: field(&info, "core"),
            debugger: info
                .get("debugger")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        // Past the handshake a call may run for as long as the tool takes.
        let _ = client.writer.set_read_timeout(None);
        let _ = client.reader.get_ref().set_read_timeout(None);
        Ok(client)
    }

    /// Who is on the other end.
    pub fn info(&self) -> &SessionInfo {
        &self.info
    }

    /// Send one request and read its answer.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.writer, "{frame}").map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())?;

        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => return Err("the session closed the connection".into()),
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
        let response: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if let Some(error) = response.get("error") {
            return Err(field(error, "message"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| "the session answered without a result".to_string())
    }
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
