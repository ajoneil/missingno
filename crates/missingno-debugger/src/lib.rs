//! A headless debugger server generic over the `missingno_core::system`
//! seam. It recognises and constructs a console through the [`factory`]
//! registry, drives it through the transport-agnostic [`Session`], and serves
//! that session over HTTP ([`http`]).
//!
//! No core-specific code lives outside the factory registry: every endpoint
//! reads a console through the seam's schema (register groups, memory regions,
//! watch conditions, the disassembly walkers), so it works over any core.

pub mod factory;
#[cfg(feature = "gb")]
pub mod gb;
pub mod http;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod session;

pub use session::{DisasmLine, Session, StopReason, validate_watch};

/// The family route extensions compiled into this build, mounted ahead of the
/// generic routes. Each declines any request it does not own.
// Each family pushes under its own feature gate, so the vec is built
// incrementally rather than from a literal.
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn extensions() -> Vec<http::Extension> {
    let mut extensions: Vec<http::Extension> = Vec::new();
    #[cfg(feature = "gb")]
    extensions.push(gb::extension);
    extensions
}

/// The family MCP tool extensions compiled into this build, offered ahead of
/// the generic tools. Each declines any tool it does not own.
#[cfg(feature = "mcp")]
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn mcp_extensions() -> Vec<mcp::McpExtension> {
    let mut extensions: Vec<mcp::McpExtension> = Vec::new();
    #[cfg(feature = "gb")]
    extensions.push(gb::mcp::extension());
    extensions
}
