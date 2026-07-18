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
/// `help` is a one-line description of what the register holds, shown on hover.
#[derive(Clone, Debug)]
pub struct Register {
    pub name: &'static str,
    pub value: u32,
    pub bits: u8,
    pub style: ValueStyle,
    pub help: Option<&'static str>,
}

impl Register {
    /// Attach a one-line description shown when the value is hovered.
    pub fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
    }
}

/// How a register value reads to a human.
#[derive(Clone, Copy, Debug)]
pub enum ValueStyle {
    Hex,
    Dec,
    Bool,
    Flags(&'static [FlagName]),
}

/// A named bit within a flags register. `help` is a one-line description of the
/// bit's meaning, shown on hover.
#[derive(Clone, Copy, Debug)]
pub struct FlagName {
    pub name: &'static str,
    pub bit: u8,
    pub help: Option<&'static str>,
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
    /// A symmetric relation over a small set of entities, one boolean per
    /// unordered pair (the TIA collision latches over the six drawn objects). A
    /// CLI renders it as a triangular table.
    Relations(PairMatrix),
    /// Label/value rows, each optionally gated by an activity pip.
    Rows(Vec<Row>),
    /// Values that count up across a hardware period (a scanline, a beam
    /// position), each shown against the period's structure.
    Sweeps(Vec<Sweep>),
    /// Palette swatch rows.
    Swatches(Vec<SwatchRow>),
    /// Pixel-buffer strips — a hardware pattern drawn cell-by-cell (a pixel
    /// FIFO, the TIA graphics registers). A CLI renders one as `▓░▓▓····`.
    Pixels(Vec<PixelStrip>),
    /// A horizontal divider.
    Rule,
}

/// One row of pixel-buffer cells: a labelled strip whose cells carry the
/// hardware's pattern at whatever colour the core can honestly resolve it to.
/// A CLI renders each strip as a run of filled/empty glyphs.
#[derive(Clone, Debug)]
pub enum PixelStrip {
    /// Cells as shade indices the frontend resolves through its user-selectable
    /// palette (DMG FIFO pixels, like the BGP/OBP swatches); `None` = an empty
    /// or transparent slot.
    Shades {
        label: &'static str,
        cells: Vec<Option<u8>>,
        help: Option<&'static str>,
    },
    /// Cells as resolved colours the core owns (CGB CRAM, TIA COLUPx); `None` =
    /// an empty or transparent slot.
    Colors {
        label: String,
        cells: Vec<Option<rgb::RGB8>>,
        help: Option<&'static str>,
    },
    /// Cells as raw on/off bits with no owned hue — the fallback where the
    /// hardware names no colour. Prefer the coloured forms otherwise.
    Bits {
        label: &'static str,
        cells: Vec<bool>,
        help: Option<&'static str>,
    },
}

/// A value sweeping a hardware period — a scanline counter over a frame, a beam
/// over a line. `value` sits in `[0, end)`; `zones` partition that period into
/// the named regions the hardware passes through (each zone runs from the
/// previous zone's end to its own `end`, the last ending at `end`). A CLI
/// renders it as `ly 91/154 (visible)`; a GUI as a number line.
#[derive(Clone, Debug)]
pub struct Sweep {
    pub label: &'static str,
    pub value: u32,
    /// Exclusive period end (e.g. 154 lines).
    pub end: u32,
    /// The period's hardware structure; empty when the period has no fixed
    /// internal boundaries to name.
    pub zones: Vec<SweepZone>,
    pub help: Option<&'static str>,
}

/// One region of a [`Sweep`]'s period, running from the previous zone's end (or
/// zero) up to `end`.
#[derive(Clone, Debug)]
pub struct SweepZone {
    pub name: &'static str,
    pub end: u32,
    pub tone: Tone,
}

impl Sweep {
    /// A sweep with no named internal structure.
    pub fn new(label: &'static str, value: u32, end: u32) -> Self {
        Sweep {
            label,
            value,
            end,
            zones: Vec::new(),
            help: None,
        }
    }

    /// Partition the period into named zones.
    pub fn zones(mut self, zones: Vec<SweepZone>) -> Self {
        self.zones = zones;
        self
    }

    /// Attach a one-line description shown when the sweep is hovered.
    pub fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
    }

    /// The zone the value currently sits in, or `None` when the period has no
    /// zones or the value is outside `[0, end)`.
    pub fn zone_at(&self, value: u32) -> Option<&SweepZone> {
        if value >= self.end {
            return None;
        }
        self.zones.iter().find(|zone| value < zone.end)
    }
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

/// A symmetric pairwise relation over a small set of entities: one boolean per
/// unordered pair. The TIA's fifteen collision latches are one such relation
/// over its six drawn objects. Rendered as a lower-triangular table.
#[derive(Clone, Debug)]
pub struct PairMatrix {
    /// The related entities, in display order.
    pub entities: &'static [&'static str],
    /// One cell per unordered pair, in canonical order: for `j` in `1..n`, for
    /// `i` in `0..j` — the pair `(i, j)`. Length is [`PairMatrix::pair_count`].
    pub cells: Vec<PairCell>,
}

/// One cell of a [`PairMatrix`]: whether the pair's relation currently holds,
/// with an optional one-line description shown on hover.
#[derive(Clone, Debug)]
pub struct PairCell {
    pub set: bool,
    pub help: Option<&'static str>,
}

/// The canonical cell index for the unordered pair of entity positions `a` and
/// `b` (with `a != b`): `j·(j−1)/2 + i` where `i < j`. Packs the lower triangle
/// row by row, so it agrees with the `for j { for i in 0..j }` build order.
pub fn pair_index(a: usize, b: usize) -> usize {
    let (i, j) = if a < b { (a, b) } else { (b, a) };
    j * (j - 1) / 2 + i
}

impl PairMatrix {
    /// The number of unordered pairs among `n` entities: `n·(n−1)/2`.
    pub fn pair_count(n: usize) -> usize {
        n * n.saturating_sub(1) / 2
    }

    /// Build a matrix, checking the cell count matches the entity count.
    pub fn new(entities: &'static [&'static str], cells: Vec<PairCell>) -> Self {
        debug_assert_eq!(cells.len(), Self::pair_count(entities.len()));
        PairMatrix { entities, cells }
    }

    /// The cell for the unordered pair of entity positions `a` and `b`.
    pub fn cell(&self, a: usize, b: usize) -> &PairCell {
        &self.cells[pair_index(a, b)]
    }
}

/// One label/value row. `active` adds an activity pip (the PPU enable rows);
/// `None` is a plain label/value row. `help` is a one-line description shown on
/// hover.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub active: Option<bool>,
    pub help: Option<&'static str>,
}

impl Row {
    /// A plain label/value row.
    pub fn value(label: impl Into<String>, value: impl Into<String>) -> Self {
        Row {
            label: label.into(),
            value: value.into(),
            active: None,
            help: None,
        }
    }

    /// A row whose label is gated by an activity pip.
    pub fn flag(label: impl Into<String>, active: bool) -> Self {
        Row {
            label: label.into(),
            value: String::new(),
            active: Some(active),
            help: None,
        }
    }

    /// Attach a one-line description shown when the row is hovered.
    pub fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
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
    use super::{MemoryWindow, PairMatrix, Sweep, SweepZone, Tone, pair_index};

    #[test]
    fn pair_index_packs_the_lower_triangle() {
        // Canonical order for six entities: for j in 1..6, for i in 0..j.
        let mut expected = 0;
        for j in 1..6 {
            for i in 0..j {
                assert_eq!(pair_index(i, j), expected);
                expected += 1;
            }
        }
        assert_eq!(expected, PairMatrix::pair_count(6));
    }

    #[test]
    fn pair_index_is_symmetric_and_covers_every_pair() {
        let n = 6;
        let mut seen = std::collections::HashSet::new();
        for a in 0..n {
            for b in 0..n {
                if a == b {
                    continue;
                }
                // Order-independent: (a,b) and (b,a) name the same cell.
                assert_eq!(pair_index(a, b), pair_index(b, a));
                seen.insert(pair_index(a, b));
            }
        }
        // Every index in 0..pair_count is hit exactly once.
        assert_eq!(seen.len(), PairMatrix::pair_count(n));
        assert!(seen.iter().all(|&i| i < PairMatrix::pair_count(n)));
    }

    #[test]
    fn pair_count_is_n_choose_2() {
        assert_eq!(PairMatrix::pair_count(0), 0);
        assert_eq!(PairMatrix::pair_count(1), 0);
        assert_eq!(PairMatrix::pair_count(2), 1);
        assert_eq!(PairMatrix::pair_count(6), 15);
    }

    fn ly_sweep(value: u32) -> Sweep {
        Sweep::new("ly", value, 154).zones(vec![
            SweepZone {
                name: "visible",
                end: 144,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "vblank",
                end: 154,
                tone: Tone::Active,
            },
        ])
    }

    #[test]
    fn zones_partition_the_period() {
        let sweep = ly_sweep(0);
        // Zones cover [0, end) contiguously with no gap or overlap.
        let mut prev_end = 0;
        for zone in &sweep.zones {
            assert!(zone.end > prev_end);
            prev_end = zone.end;
        }
        assert_eq!(prev_end, sweep.end);
    }

    #[test]
    fn zone_at_maps_value_to_its_region() {
        let sweep = ly_sweep(0);
        assert_eq!(sweep.zone_at(0).map(|z| z.name), Some("visible"));
        assert_eq!(sweep.zone_at(143).map(|z| z.name), Some("visible"));
        assert_eq!(sweep.zone_at(144).map(|z| z.name), Some("vblank"));
        assert_eq!(sweep.zone_at(153).map(|z| z.name), Some("vblank"));
        // Boundary is exclusive: the period end and beyond fall in no zone.
        assert_eq!(sweep.zone_at(154).map(|z| z.name), None);
        assert_eq!(sweep.zone_at(200).map(|z| z.name), None);
    }

    #[test]
    fn zoneless_sweep_has_no_region() {
        let sweep = Sweep::new("lx", 42, 114);
        assert_eq!(sweep.zone_at(42).map(|z| z.name), None);
    }

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
