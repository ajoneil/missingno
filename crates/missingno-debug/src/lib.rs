//! Debug sidecar formats, independent of the console being debugged.
//!
//! Symbol tables and code/data logs are conventions of the tools around an
//! emulator rather than of any one machine: the `.sym` grammar and the Mesen
//! CDL flag bits mean the same thing whichever CPU produced them. What differs
//! per console — how a CPU address maps onto a ROM offset, what state a
//! watchpoint can name — stays with that console.

pub mod cdl;
pub mod symbols;
