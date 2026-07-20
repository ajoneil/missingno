//! A headless debugger server over a `missingno-session` session. It recognises
//! and constructs a console through that crate's factory registry, hosts it in a
//! session, and serves that session over one of two transports — HTTP ([`http`])
//! for scripted and bulk access, or MCP-over-stdio ([`mcp`]) for interactive
//! agent use. Either transport is a client of the session, so an agent can drive
//! a machine this process hosts or one another process published for attaching.
//!
//! Both transports are purely generic: every readout and command reaches the
//! machine through the session, which reads the console through the seam's schema
//! (register groups, sidebar sections, memory regions, watch conditions, the
//! graphics and waveform surfaces, the disassembly walkers), so they work over
//! any core with no core-specific code outside the factory registry.

pub mod http;
#[cfg(feature = "mcp")]
pub mod mcp;
