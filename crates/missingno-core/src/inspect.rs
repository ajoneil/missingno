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

// --- Sidebar schema ----------------------------------------------------------
//
// A console-agnostic description of the debugger's left-column sidebar: a stack
// of collapsible sections, each carrying typed blocks (register files, pointer
// rows, a bit table, palette swatches). A family builds these from one shared
// builder serving both the live console (paused) and the per-vblank snapshot
// (running), so the two agree by construction. The frontend renders them; the
// core names no colours.

/// One collapsible sidebar section — a heading, a one-line summary for the
/// collapsed state, an optional activity indicator, an optional inline detail,
/// and the typed content blocks shown when expanded.
#[derive(Clone, Debug)]
pub struct Section {
    pub name: &'static str,
    pub summary: String,
    /// Drives the header activity pip: `Some(true)` lit, `Some(false)` dim,
    /// `None` no pip.
    pub active: Option<bool>,
    /// An accented detail shown beside the heading (the PPU mode).
    pub detail: Option<Detail>,
    pub blocks: Vec<SectionBlock>,
}

/// An accented one-line detail beside a section heading.
#[derive(Clone, Debug)]
pub struct Detail {
    pub text: String,
    pub tone: Tone,
}

/// A semantic accent class for a [`Detail`]. The frontend maps each to a
/// palette colour — the core never names colours. Derived from the only
/// coloured detail the Game Boy sidebar shows, the PPU mode: its four modes
/// map onto `Idle`/`Active`/`Scanning`/`Rendering`, with `Neutral` for an
/// unaccented detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    /// Between-work (PPU HBlank).
    Idle,
    /// At a frame/field boundary (PPU VBlank).
    Active,
    /// Scanning inputs (PPU OAM scan).
    Scanning,
    /// Producing output (PPU drawing).
    Rendering,
    /// Requested but not yet serviced (an interrupt-flag bit that is set while
    /// the corresponding enable may not be).
    Pending,
}

/// One typed block within a [`Section`].
#[derive(Clone, Debug)]
pub enum SectionBlock {
    /// A flat register file; a `Flags`-styled register renders as bit chips.
    Registers(RegisterGroup),
    /// 8-bit register halves with their combined 16-bit value.
    Pairs(Vec<RegisterPair>),
    /// Pointer-style rows (pc/sp).
    Pointers(Vec<Pointer>),
    /// The interrupt-table shape: named bit columns, one row per source
    /// register, an optional corner master flag (IME).
    Table(BitTable),
    /// Label/value rows, each optionally gated by an activity pip.
    Rows(Vec<Row>),
    /// Palette swatch rows.
    Swatches(Vec<SwatchRow>),
    /// A horizontal divider.
    Rule,
}

/// A pointer-style register row (a pc or sp), optionally carrying whether the
/// pointer is currently advancing. `active: Some(false)` marks a stalled
/// pointer — a halted CPU's program counter is not moving; `active: None` means
/// the pointer has no advancing/stalled concept (the stack pointer).
#[derive(Clone, Debug)]
pub struct Pointer {
    pub register: Register,
    pub active: Option<bool>,
}

/// A register split into its two 8-bit halves, with the combined value derived
/// from them. The low half may carry a `Flags` style (the SM83 `f`).
#[derive(Clone, Debug)]
pub struct RegisterPair {
    pub high: Register,
    pub low: Register,
}

impl RegisterPair {
    /// The combined value: the high half shifted above the low half's width.
    pub fn combined(&self) -> u32 {
        (self.high.value << self.low.bits) | self.low.value
    }
}

/// A table of named bit columns with one row per source register — the
/// interrupt table (IE/IF over the five sources), or a PPU control register
/// decoded into its named bits.
#[derive(Clone, Debug)]
pub struct BitTable {
    pub columns: Vec<BitColumn>,
    /// A master flag shown in the table's corner (IME), if any.
    pub corner: Option<Flag>,
    pub rows: Vec<BitRow>,
}

/// One column of a [`BitTable`]: its header name and, when the column stands
/// for a hardware concept other consoles also expose, that [`Concept`] so the
/// renderer can show a shared symbol for it.
#[derive(Clone, Copy, Debug)]
pub struct BitColumn {
    pub name: &'static str,
    pub concept: Option<Concept>,
}

impl BitColumn {
    /// A column with no shared hardware concept — rendered by its name alone.
    pub fn plain(name: &'static str) -> Self {
        BitColumn {
            name,
            concept: None,
        }
    }

    /// A column standing for a shared hardware [`Concept`].
    pub fn concept(name: &'static str, concept: Concept) -> Self {
        BitColumn {
            name,
            concept: Some(concept),
        }
    }
}

/// A hardware concept a [`BitColumn`] stands for, drawn from the interrupt
/// sources several consoles share so a renderer can give each a common symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concept {
    /// The vertical-blanking interval.
    VBlank,
    /// A video/display status condition (the Game Boy STAT sources).
    VideoStatus,
    /// A hardware timer.
    Timer,
    /// A serial-link transfer.
    Serial,
    /// A player-input event (the Game Boy joypad).
    Input,
}

/// One row of a [`BitTable`]: a name, one bool per column, and a [`Tone`]
/// classing the row's meaning (an enabled mask versus a pending-flag mask).
#[derive(Clone, Debug)]
pub struct BitRow {
    pub name: &'static str,
    pub bits: Vec<bool>,
    pub tone: Tone,
}

/// A named boolean shown as a lit/dim badge.
#[derive(Clone, Copy, Debug)]
pub struct Flag {
    pub name: &'static str,
    pub active: bool,
}

/// One label/value row. `active` adds an activity pip (the PPU enable rows);
/// `None` is a plain label/value row.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub active: Option<bool>,
}

impl Row {
    /// A plain label/value row.
    pub fn value(label: impl Into<String>, value: impl Into<String>) -> Self {
        Row {
            label: label.into(),
            value: value.into(),
            active: None,
        }
    }

    /// A row whose label is gated by an activity pip.
    pub fn flag(label: impl Into<String>, active: bool) -> Self {
        Row {
            label: label.into(),
            value: String::new(),
            active: Some(active),
        }
    }
}

/// A palette swatch row.
#[derive(Clone, Debug)]
pub enum SwatchRow {
    /// A packed shade byte the frontend resolves through its user-selectable
    /// palette (the DMG BGP/OBP registers); the core never picks the display
    /// colours.
    Shades { label: &'static str, packed: u8 },
    /// Resolved colours the core genuinely owns (CGB palette RAM).
    Colors {
        label: String,
        colors: Vec<rgb::RGB8>,
    },
}

/// The fallback sidebar for a core that has not built family-specific sections:
/// its register groups as one CPU section, summarised by the program counter.
pub fn default_sections(groups: Vec<RegisterGroup>) -> Vec<Section> {
    let summary = groups
        .iter()
        .flat_map(|group| &group.registers)
        .find(|register| register.name == "pc")
        .map(|pc| {
            let digits = (pc.bits as usize).div_ceil(4);
            format!("pc {:0width$X}", pc.value, width = digits)
        })
        .unwrap_or_default();
    let blocks = groups.into_iter().map(SectionBlock::Registers).collect();
    vec![Section {
        name: "CPU",
        summary,
        active: None,
        detail: None,
        blocks,
    }]
}

/// A contiguous span of the CPU-visible address space, named by its role.
#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub name: &'static str,
    pub start: u32,
    pub len: u32,
}

/// A copied span of address space: a base address and the bytes read upward
/// from it. Backs the memory viewer's running-mode window (what a per-vblank
/// snapshot could capture) and the Game Boy's PC-anchored disassembly capture.
#[derive(Clone, Debug)]
pub struct MemoryWindow {
    pub base: u32,
    pub bytes: Vec<u8>,
}

impl MemoryWindow {
    /// One past the last captured address.
    pub fn end(&self) -> u32 {
        self.base.saturating_add(self.bytes.len() as u32)
    }

    /// Whether `address` falls within the captured span.
    pub fn contains(&self, address: u32) -> bool {
        address >= self.base && address < self.end()
    }

    /// The captured byte at `address`, or `None` outside the span.
    pub fn read(&self, address: u32) -> Option<u8> {
        address
            .checked_sub(self.base)
            .and_then(|offset| self.bytes.get(offset as usize).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryWindow;

    fn window() -> MemoryWindow {
        MemoryWindow {
            base: 0xC100,
            bytes: vec![0x10, 0x20, 0x30, 0x40],
        }
    }

    #[test]
    fn contains_spans_base_to_end() {
        let w = window();
        assert!(!w.contains(0xC0FF));
        assert!(w.contains(0xC100));
        assert!(w.contains(0xC103));
        assert!(!w.contains(0xC104));
    }

    #[test]
    fn read_returns_byte_in_span_and_none_outside() {
        let w = window();
        assert_eq!(w.read(0xC100), Some(0x10));
        assert_eq!(w.read(0xC103), Some(0x40));
        assert_eq!(w.read(0xC104), None);
        assert_eq!(w.read(0xC0FF), None);
    }
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
