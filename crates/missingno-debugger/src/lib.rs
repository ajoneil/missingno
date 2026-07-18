//! A headless debugger server generic over the `missingno_core::system`
//! seam. It recognises and constructs a console through the [`factory`]
//! registry, drives it through the transport-agnostic [`Session`], and serves
//! that session over HTTP ([`http`]).
//!
//! No core-specific code lives outside the factory registry: every endpoint
//! reads a console through the seam's schema (register groups, memory regions,
//! watch conditions, the disassembly walkers), so it works over any core.

pub mod factory;
pub mod http;
pub mod session;

pub use session::{DisasmLine, Session, StopReason, validate_watch};
