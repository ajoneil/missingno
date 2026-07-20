//! A headless debugger server generic over the `missingno_core::system`
//! seam. It recognises and constructs a console through the [`factory`]
//! registry, drives it through the transport-agnostic [`Session`], and serves
//! that session over one of two transports — HTTP ([`http`]) for scripted and
//! bulk access, or MCP-over-stdio ([`mcp`]) for interactive agent use.
//!
//! Both transports are purely generic: every readout and command is a
//! [`Session`] call reading the console through the seam's schema (register
//! groups, sidebar sections, memory regions, watch conditions, the graphics and
//! waveform surfaces, the disassembly walkers), so they work over any core with
//! no core-specific code outside the factory registry.

//! A session host can additionally publish the session it owns on a Unix
//! socket ([`attach`]), so a client in another process drives that live machine
//! instead of creating one — the app's own window is then just another client
//! of it.

#[cfg(all(unix, feature = "mcp"))]
pub mod attach;
pub mod factory;
pub mod http;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod session;
pub mod shared;

#[cfg(all(unix, feature = "mcp"))]
pub use attach::{AttachClient, AttachEndpoint, Publication, SessionInfo};
pub use session::{DisasmLine, Session, StopReason, validate_watch};
pub use shared::{
    AudioSink, ExtractedMachine, MemoryInterest, RunningReadout, SessionEvent, SessionHandle,
    SharedSession,
};
