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
//!
//! [`SocketHost`] is that hosting, without the session: any frontend that wants
//! to answer newline JSON-RPC in the same runtime directory — the app's
//! UI-automation surface, the curator's — supplies a [`HostSpec`] and a frame
//! handler and gets the same directory, permissions, refusal and shutdown
//! semantics.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use crate::shared::SessionHandle;
use crate::tools;

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

// --- the shared socket host ---------------------------------------------------

/// What distinguishes one socket host from another: what it calls itself in the
/// filesystem, in a refusal and in a thread listing, and what it makes of a
/// frame split across a read timeout.
#[derive(Clone, Copy)]
pub struct HostSpec {
    /// The socket file's stem; the host's process id completes the name.
    pub socket_prefix: &'static str,
    /// How a host already answering on the same path is named when a second one
    /// refuses to take it over.
    pub occupant: &'static str,
    pub accept_thread: &'static str,
    pub client_thread: &'static str,
    pub partial_frames: PartialFrames,
}

/// What a read timeout landing mid-frame does with the bytes already read.
#[derive(Clone, Copy, PartialEq)]
pub enum PartialFrames {
    /// Keep them; the rest of the frame joins them on a later read.
    Resumed,
    /// Drop them; whatever arrives next is read as a frame of its own.
    Dropped,
}

/// True for as long as the host accepts clients, false once it closes. A handler
/// that parks a call watches it, so a parked wait cannot outlast the endpoint.
#[derive(Clone)]
pub struct Serving(Arc<AtomicBool>);

impl Serving {
    pub fn is_serving(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One request frame's fields, for a handler to dispatch on.
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

impl Request {
    /// Read a line as a request frame; the caller words the parse failure.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        let message: Value = serde_json::from_str(line)?;
        Ok(Request {
            id: message.get("id").cloned().unwrap_or(Value::Null),
            method: message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            params: message.get("params").cloned().unwrap_or_else(|| json!({})),
        })
    }
}

pub fn success_frame(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_frame(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

/// A published endpoint: the listening socket plus the thread accepting clients.
/// Dropping it stops accepting and removes the socket file, so a host that ends
/// leaves nothing behind for a discovering client to trip over.
pub struct SocketHost {
    path: PathBuf,
    serving: Serving,
    thread: Option<JoinHandle<()>>,
}

impl SocketHost {
    /// Publish in `dir`, creating the directory user-only if it does not exist,
    /// and answer each client's lines with `frames`. A socket file left by a dead
    /// host of the same name is replaced; one whose host still answers is an
    /// error rather than a silent takeover.
    pub fn open_in<F>(dir: &Path, spec: HostSpec, frames: F) -> std::io::Result<Self>
    where
        F: Fn(&str, &Serving) -> Option<Value> + Clone + Send + 'static,
    {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;

        let path = dir.join(format!(
            "{}-{}.sock",
            spec.socket_prefix,
            std::process::id()
        ));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("{} already answers on {}", spec.occupant, path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let serving = Serving(Arc::new(AtomicBool::new(true)));
        let thread = std::thread::Builder::new()
            .name(spec.accept_thread.into())
            .spawn({
                let serving = serving.clone();
                move || accept_loop(listener, spec, frames, serving)
            })?;

        Ok(SocketHost {
            path,
            serving,
            thread: Some(thread),
        })
    }

    /// Where this host is published, for a frontend that wants to show the user.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketHost {
    fn drop(&mut self) {
        self.serving.0.store(false, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop<F>(listener: UnixListener, spec: HostSpec, frames: F, serving: Serving)
where
    F: Fn(&str, &Serving) -> Option<Value> + Clone + Send + 'static,
{
    let mut clients: Vec<JoinHandle<()>> = Vec::new();
    while serving.is_serving() {
        match listener.accept() {
            Ok((stream, _)) => {
                let frames = frames.clone();
                let serving = serving.clone();
                if let Ok(thread) = std::thread::Builder::new()
                    .name(spec.client_thread.into())
                    .spawn(move || serve_client(stream, spec.partial_frames, frames, serving))
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
/// disconnect ends this thread only — the host and its other clients are
/// untouched.
fn serve_client<F>(stream: UnixStream, partial: PartialFrames, frames: F, serving: Serving)
where
    F: Fn(&str, &Serving) -> Option<Value>,
{
    // A read timeout is what lets a quiet client notice the endpoint closing.
    let _ = stream.set_read_timeout(Some(ACCEPT_POLL));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while serving.is_serving() {
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if partial == PartialFrames::Dropped {
                    line.clear();
                }
                continue;
            }
            Err(_) => return,
        }
        let response = if line.trim().is_empty() {
            None
        } else {
            frames(&line, &serving)
        };
        line.clear();
        let Some(response) = response else { continue };
        if writeln!(writer, "{response}").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

// --- the session's host -------------------------------------------------------

const SESSION_HOST: HostSpec = HostSpec {
    socket_prefix: "session",
    occupant: "a session",
    accept_thread: "attach-endpoint",
    client_thread: "attach-client",
    partial_frames: PartialFrames::Dropped,
};

/// A published session: a socket host answering the session tool vocabulary.
pub struct AttachEndpoint(SocketHost);

impl AttachEndpoint {
    /// Publish `handle` in the default runtime directory.
    pub fn open(handle: SessionHandle, publication: Publication) -> std::io::Result<Self> {
        Self::open_in(&runtime_dir(), handle, publication)
    }

    /// Publish `handle` in `dir`.
    pub fn open_in(
        dir: &Path,
        handle: SessionHandle,
        publication: Publication,
    ) -> std::io::Result<Self> {
        SocketHost::open_in(dir, SESSION_HOST, move |line, _| {
            Some(answer(line, &handle, &publication))
        })
        .map(AttachEndpoint)
    }

    /// Where this session is published, for a host that wants to show the user.
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Dispatch one request frame against the hosted session.
fn answer(line: &str, handle: &SessionHandle, publication: &Publication) -> Value {
    let Request { id, method, params } = match Request::parse(line) {
        Ok(request) => request,
        Err(error) => return error_frame(Value::Null, &format!("parse error: {error}")),
    };

    match method.as_str() {
        "session/info" => success_frame(
            id,
            json!({
                "pid": std::process::id(),
                "title": publication.title,
                "core": publication.core,
                "debugger": handle.is_debugger(),
            }),
        ),
        "tools/list" => success_frame(id, tools::session_tools_json(handle)),
        "tools/call" => success_frame(
            id,
            tools::call_session_tool_json(handle, &publication.core, &params),
        ),
        other => error_frame(id, &format!("method not found: {other}")),
    }
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
