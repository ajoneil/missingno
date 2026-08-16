//! The TMS9918A at register level: where the raster stands, what the status
//! latches hold, the five tables R2-R6 point at, and the backdrop R7 paints.

use missingno_core::inspect::{
    BitColumn, BitRow, BitTable, ColorSwatch, Concept, Detail, Row, Section, SectionBlock,
    SwatchRow, Sweep, SweepZone, Tone,
};
use missingno_ti_vdp::{ACTIVE_LINES, Mode, Standard, Vdp};

use super::Sg1000InspectState;
use super::palette::ti_colour;

/// Dots per counter line; the chip states the windows inside the raster.
const DOTS_PER_LINE: u32 = 342;
/// The status register's low five bits carry the sprite-scan counter.
const SCAN_COUNTER_MASK: u8 = 0x1F;

/// What the chip's own accessors make of the register file: the mode R0/R1
/// select, the five table bases R2-R6 point at, R1's sprite geometry, and the
/// backdrop R7 selects.
#[derive(Clone)]
pub struct VdpLayout {
    pub mode: Mode,
    pub name_table: u16,
    pub pattern_table: u16,
    pub colour_table: u16,
    pub sprite_attributes: u16,
    pub sprite_patterns: u16,
    pub sprites_16x16: bool,
    pub magnified: bool,
    pub backdrop: u8,
}

impl VdpLayout {
    pub(crate) fn of(vdp: &Vdp) -> VdpLayout {
        VdpLayout {
            mode: vdp.mode(),
            name_table: vdp.name_table_base(),
            pattern_table: vdp.pattern_table_base(),
            colour_table: vdp.colour_table_base(),
            sprite_attributes: vdp.sprite_attribute_base(),
            sprite_patterns: vdp.sprite_pattern_base(),
            sprites_16x16: vdp.sprites_16x16(),
            magnified: vdp.magnified(),
            backdrop: vdp.backdrop(),
        }
    }
}

pub(crate) fn section(state: &Sg1000InspectState) -> Section {
    let layout = &state.vdp_layout;
    let registers = state
        .vdp_registers
        .iter()
        .enumerate()
        .map(|(index, &value)| Row::value(format!("r{index}"), format!("{value:02X}")))
        .collect();
    // The dot cycle within a line carries no named zones.
    let dot =
        Sweep::new("dot", state.dot as u32, DOTS_PER_LINE).help("VDP dot within the scanline");
    Section {
        name: "VDP",
        summary: format!("line {} · dot {}", state.line, state.dot),
        active: None,
        detail: Some(mode_detail(layout.mode)),
        blocks: vec![
            SectionBlock::Sweeps(vec![line_sweep(state.line, state.standard), dot]),
            SectionBlock::Rule,
            SectionBlock::Table(status_table(state.vdp_status)),
            SectionBlock::Rows(vec![
                Row::value(
                    "scan",
                    format!("{:02}", state.vdp_status & SCAN_COUNTER_MASK),
                )
                .help("sprite-scan counter (status bits 0-4) — the entry the scan halted on"),
            ]),
            SectionBlock::Rule,
            SectionBlock::Rows(table_rows(layout)),
            SectionBlock::Rule,
            SectionBlock::Rows(vec![
                Row::flag("16x16", layout.sprites_16x16)
                    .help("R1 SIZE — four generators to a sprite"),
                Row::flag("magnified", layout.magnified)
                    .help("R1 MAG — sprites drawn at double size"),
            ]),
            SectionBlock::Rule,
            backdrop_swatches(layout.backdrop),
            SectionBlock::Rule,
            SectionBlock::Rows(registers),
        ],
    }
}

/// The line counter across the frame. The visible raster is not contiguous in
/// counter order — the top border rides the wrap — so the border shows as two
/// zones with the blanking lines between them.
fn line_sweep(line: u16, standard: Standard) -> Sweep {
    let lines_per_frame = standard.lines_per_frame() as u32;
    let display = ACTIVE_LINES as u32;
    let bottom = display + standard.bottom_border() as u32;
    let top = lines_per_frame - standard.top_border() as u32;
    Sweep::new("line", line as u32, lines_per_frame)
        .zones(vec![
            SweepZone {
                name: "display",
                end: display,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "border",
                end: bottom,
                tone: Tone::Idle,
            },
            SweepZone {
                name: "blank",
                end: top,
                tone: Tone::Active,
            },
            SweepZone {
                name: "border",
                end: lines_per_frame,
                tone: Tone::Idle,
            },
        ])
        .help("VDP line counter — 192 display lines, the borders, and the blanking between them")
}

/// The mode as the heading's accent. The four the Data Manual defines have a
/// stated table layout; the M1/M2/M3 combinations it leaves out have none, so
/// they carry no accent.
fn mode_detail(mode: Mode) -> Detail {
    let (text, tone) = match mode {
        Mode::GraphicsI => ("Graphics I", Tone::Rendering),
        Mode::GraphicsII => ("Graphics II", Tone::Rendering),
        Mode::Multicolor => ("Multicolor", Tone::Rendering),
        Mode::Text => ("Text", Tone::Rendering),
        Mode::BitmapText => ("Bitmap Text", Tone::Neutral),
        Mode::BitmapMulticolor => ("Bitmap Multicolor", Tone::Neutral),
        Mode::TextMulticolor => ("Text Multicolor", Tone::Neutral),
    };
    Detail {
        text: text.to_string(),
        tone,
    }
}

/// Where the five tables sit in VRAM — the addresses R2-R6 select.
fn table_rows(layout: &VdpLayout) -> Vec<Row> {
    vec![
        Row::value("name", address(layout.name_table)).help("name table base (R2)"),
        Row::value("pattern", address(layout.pattern_table))
            .help("pattern generator base (R4); the bitmap modes take only its half select"),
        Row::value("colour", address(layout.colour_table))
            .help("colour table base (R3); the bitmap modes mask their fetches with R3 instead"),
        Row::value("sprite attr", address(layout.sprite_attributes))
            .help("sprite attribute table base (R5)"),
        Row::value("sprite gen", address(layout.sprite_patterns))
            .help("sprite generator base (R6)"),
    ]
}

fn address(base: u16) -> String {
    format!("{base:04X}")
}

/// The backdrop R7's low nibble selects, resolved through the datasheet
/// palette the console presents indices with.
fn backdrop_swatches(index: u8) -> SectionBlock {
    SectionBlock::Swatches(vec![SwatchRow::Colors {
        label: "backdrop".to_string(),
        colors: vec![ColorSwatch {
            color: ti_colour(index),
            raw: Some(index as u16),
        }],
    }])
}

/// The status register's three flags; the low five bits carry the sprite-scan
/// counter, which is a number rather than a bit and sits in its own row.
fn status_table(status: u8) -> BitTable {
    BitTable {
        columns: vec![
            // F is the chip's vertical-blank interrupt source.
            BitColumn::concept("f", Concept::VBlank),
            BitColumn::concept("5s", Concept::SpriteOverflow),
            BitColumn::concept("c", Concept::SpriteCollision),
        ],
        corner: None,
        rows: vec![BitRow {
            name: "status",
            bits: vec![status & 0x80 != 0, status & 0x40 != 0, status & 0x20 != 0],
            tone: Tone::Neutral,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::fixtures::{power_on_state, rows, value_of};

    /// A chip whose register file has been written through the control port.
    fn vdp_with(registers: [u8; 8]) -> Vdp {
        let mut vdp = Vdp::new(Standard::Ntsc);
        for (index, &value) in registers.iter().enumerate() {
            vdp.write_control(value);
            vdp.write_control(0x80 | index as u8);
        }
        vdp
    }

    #[test]
    fn vdp_section_reports_all_eight_registers() {
        let state = Sg1000InspectState {
            vdp_registers: [0x00, 0x60, 0x0E, 0xFF, 0x03, 0x76, 0x03, 0x01],
            ..power_on_state()
        };
        let section = section(&state);
        let rows = rows(&section);
        for (index, value) in ["00", "60", "0E", "FF", "03", "76", "03", "01"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(value_of(&rows, &format!("r{index}")), Some(value));
        }
    }

    #[test]
    fn vdp_section_shows_the_tables_the_registers_select() {
        let vdp = vdp_with([0x00, 0x63, 0x0E, 0xFF, 0x03, 0x76, 0x03, 0x01]);
        let state = Sg1000InspectState {
            vdp_registers: *vdp.registers(),
            vdp_layout: VdpLayout::of(&vdp),
            ..power_on_state()
        };
        let section = section(&state);
        let rows = rows(&section);
        assert_eq!(value_of(&rows, "name"), Some("3800"));
        assert_eq!(value_of(&rows, "pattern"), Some("1800"));
        assert_eq!(value_of(&rows, "colour"), Some("3FC0"));
        assert_eq!(value_of(&rows, "sprite attr"), Some("3B00"));
        assert_eq!(value_of(&rows, "sprite gen"), Some("1800"));
        // R1 $63 selects 16×16 sprites, magnified, in Graphics I.
        assert_eq!(
            section.detail.as_ref().map(|detail| detail.text.as_str()),
            Some("Graphics I")
        );
        for label in ["16x16", "magnified"] {
            assert_eq!(
                rows.iter()
                    .find(|row| row.label == label)
                    .and_then(|row| row.active),
                Some(true)
            );
        }
    }
}
