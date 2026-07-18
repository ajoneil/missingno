//! Console-agnostic schema for a core's observable internals: register groups,
//! the CPU-visible memory map, and the watch conditions a core can name.
//!
//! Registers panes, memory viewers, watch UIs, and the headless server read a
//! core through these types, so they work over any core without knowing its
//! hardware. A core's backend fills them in from its own state.

/// A named set of related registers — a CPU file, a PPU block.
#[derive(Clone, Debug)]
pub struct RegisterGroup {
    pub name: &'static str,
    pub registers: Vec<Register>,
}

/// One register's current value with the width and presentation to render it.
#[derive(Clone, Debug)]
pub struct Register {
    pub name: &'static str,
    pub value: u32,
    pub bits: u8,
    pub style: ValueStyle,
}

/// How a register value reads to a human.
#[derive(Clone, Copy, Debug)]
pub enum ValueStyle {
    Hex,
    Dec,
    Bool,
    Flags(&'static [FlagName]),
}

/// A named bit within a flags register.
#[derive(Clone, Copy, Debug)]
pub struct FlagName {
    pub name: &'static str,
    pub bit: u8,
}

/// A contiguous span of the CPU-visible address space, named by its role.
#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub name: &'static str,
    pub start: u32,
    pub len: u32,
}

/// A watchable quantity a core exposes, with the parameter its watch takes.
#[derive(Clone, Copy, Debug)]
pub struct Watchable {
    pub key: &'static str,
    pub label: &'static str,
    pub param: WatchParam,
}

/// What a watchable's condition is parameterised by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchParam {
    None,
    Address,
    Value { bits: u8 },
    AddressValue,
}

/// One condition within a watch: a watchable key and its parameter values.
/// `key` is owned because a watch round-trips through UIs and HTTP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchTerm {
    pub key: String,
    pub address: Option<u32>,
    pub value: Option<u32>,
}

/// A watch: a conjunction of terms that fires when every term holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub terms: Vec<WatchTerm>,
}

impl Watch {
    /// The common single-term watch.
    pub fn single(key: impl Into<String>, address: Option<u32>, value: Option<u32>) -> Self {
        Watch {
            terms: vec![WatchTerm {
                key: key.into(),
                address,
                value,
            }],
        }
    }
}
