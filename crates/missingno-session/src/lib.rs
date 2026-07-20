//! The session component: one owner of an emulated machine, hosted on its own
//! thread, that every consumer drives as a client.
//!
//! [`SharedSession`] holds the machine — a debugger-hosting [`Session`] or a
//! plain console — runs the paced free-run loop, publishes the latest frame,
//! status and inspection snapshot, and serializes every client's commands.
//! [`SessionHandle`] is the cloneable client through which all access flows.
//! The machine is built from media by the [`factory`] registry, the one point
//! that knows concrete cores.
//!
//! Everything here is transport-free: an embedder depending on this crate alone
//! gets a running, inspectable machine with no server linked. The `tools`
//! feature adds the session's agent tool surface ([`tools`]) and the Unix-socket
//! [`attach`] endpoint that publishes it to another process; the HTTP and
//! MCP-over-stdio servers live in `missingno-debugger`, which is a client of
//! this crate like any other.

#[cfg(all(unix, feature = "tools"))]
pub mod attach;
pub mod factory;
pub mod session;
pub mod shared;
#[cfg(feature = "tools")]
pub mod tools;

#[cfg(all(unix, feature = "tools"))]
pub use attach::{AttachClient, AttachEndpoint, Publication, SessionInfo};
pub use session::{DisasmLine, Session, StopReason, validate_watch};
pub use shared::{
    AudioSink, ExtractedMachine, MemoryInterest, RunningReadout, SessionEvent, SessionHandle,
    SharedSession,
};
