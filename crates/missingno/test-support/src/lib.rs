//! The harness pieces every core's test suite would otherwise re-type: loading
//! a reference image, diffing a rendered surface against it, driving a
//! self-checking ROM to its verdict, fetching a chip's oracle set, and the
//! save-state and recording round-trip shapes.
//!
//! A dev-only crate — nothing here ships. Each piece stays deliberately
//! unopinionated about what a suite asserts: dimensions, tolerances and
//! verdict handling are the caller's, so adopting it changes no test's
//! meaning.

pub mod compare;
pub mod oracle;
pub mod reference;
pub mod roundtrip;
pub mod verdict;
