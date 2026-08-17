//! The VCS sidebar sections, built from the inspection state alone so the live
//! debugger (paused) and the per-frame snapshot agree by construction.

use missingno_core::inspect;
use rgb::RGB8;

use super::inspect::VcsInspectState;

/// The 6507 register file and the TIA/RIOT state the inspection struct carries,
/// in reading order: what runs, what it draws, what it sounds like, what it
/// reads back, and what it runs from.
pub fn vcs_sidebar_sections(state: &VcsInspectState) -> Vec<inspect::Section> {
    vec![
        inspect::cpu_section(crate::debugger::cpu_register_groups(
            state.pc, state.a, state.x, state.y, state.s, state.p,
        )),
        tia_section(state),
        audio_section(state),
        riot_section(state),
        cartridge_section(state),
    ]
}

/// The Cartridge section: the board, its selected bank on a banked board, and —
/// on a DPC cart — the custom chip's data fetchers, music voices and RNG.
fn cartridge_section(state: &VcsInspectState) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let cart = &state.cartridge;
    let summary = match cart.bank {
        Some(bank) => format!("{} · bank {bank}", cart.board),
        None => cart.board.to_owned(),
    };

    let mut rows = vec![Row::value("board", cart.board).help("cartridge board type")];
    if let Some(bank) = cart.bank {
        rows.push(Row::value("bank", bank.to_string()).help("4 KB ROM bank in the window"));
    }
    let mut blocks = vec![SectionBlock::Rows(rows)];

    if let Some(dpc) = &cart.dpc {
        blocks.push(SectionBlock::Rule);
        let fetcher_rows = dpc
            .fetchers
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mode = if f.music {
                    if f.oscillator { " music/osc" } else { " music" }
                } else {
                    ""
                };
                Row {
                    label: format!("df{i}"),
                    value: format!(
                        "ptr {:03X} top {:02X} bot {:02X}{mode}",
                        f.counter, f.top, f.bottom
                    ),
                    active: Some(f.flag),
                    help: Some("data fetcher: display-ROM pointer, top/bottom limits, flag pip"),
                }
            })
            .collect();
        blocks.push(SectionBlock::Rows(fetcher_rows));
        blocks.push(SectionBlock::Rule);
        blocks.push(SectionBlock::Rows(vec![
            Row::value("rng", format!("{:02X}", dpc.rng))
                .help("8-bit LFSR, clocked on every select"),
            Row::value("prog bank", dpc.bank.to_string()).help("F8-banked program ROM window"),
        ]));
    }

    inspect::Section {
        name: "Cartridge",
        summary,
        active: None,
        detail: None,
        blocks,
    }
}

/// A player's GRP as 8 screen-ordered cells: bit 7 draws leftmost, unless REFP
/// mirrors the pattern and bit 0 leads. Mirrors [`Player::output`]'s bit select.
fn player_pattern_bits(graphics: u8, reflect: bool) -> [bool; 8] {
    std::array::from_fn(|i| {
        let bit = if reflect { i as u8 } else { 7 - i as u8 };
        graphics & (1 << bit) != 0
    })
}

/// The playfield's 20-cell left-half pattern, left to right: PF0's high nibble
/// (bit 4 first), PF1 (bit 7 first), PF2 (bit 0 first). Mirrors the
/// `Playfield::pixel` cell decode. The right half repeats or reflects this per
/// CTRLPF, shown as the `mirror` flag rather than a second 20 cells.
fn playfield_cells(pf0: u8, pf1: u8, pf2: u8) -> [bool; 20] {
    std::array::from_fn(|cell| match cell {
        0..=3 => pf0 & (0x10 << cell) != 0,
        4..=11 => pf1 & (0x80 >> (cell - 4)) != 0,
        _ => pf2 & (0x01 << (cell - 12)) != 0,
    })
}

/// The TIA graphics strips: the two players in their COLUP0/COLUP1 hue, the
/// playfield in COLUPF, and the missile/ball enables as single cells in the
/// object's hue. A clear pattern bit draws nothing — the object drives no pixel
/// there and the priority mux falls through — and renders as an unlit cell.
fn tia_graphics_block(state: &VcsInspectState) -> inspect::SectionBlock {
    use inspect::PixelStrip;

    let lit = |bits: &[bool], color: RGB8| -> Vec<Option<RGB8>> {
        bits.iter().map(|&b| b.then_some(color)).collect()
    };

    inspect::SectionBlock::Pixels(vec![
        PixelStrip::Colors {
            label: "pf".to_owned(),
            cells: lit(
                &playfield_cells(state.pf0, state.pf1, state.pf2),
                state.color_pf,
            ),
            help: Some(
                "playfield left half (PF0/PF1/PF2) in COLUPF; right half per mirror; clear bits draw nothing",
            ),
        },
        PixelStrip::Colors {
            label: "grp0".to_owned(),
            cells: lit(
                &player_pattern_bits(state.grp0, state.grp0_reflect),
                state.color_p0,
            ),
            help: Some(
                "player 0 graphics (GRP0) in COLUP0; bit 7 at left, REFP0 mirrors; clear bits draw nothing",
            ),
        },
        PixelStrip::Colors {
            label: "grp1".to_owned(),
            cells: lit(
                &player_pattern_bits(state.grp1, state.grp1_reflect),
                state.color_p1,
            ),
            help: Some(
                "player 1 graphics (GRP1) in COLUP1; bit 7 at left, REFP1 mirrors; clear bits draw nothing",
            ),
        },
        PixelStrip::Colors {
            label: "m0".to_owned(),
            cells: vec![state.missile0.then_some(state.color_p0)],
            help: Some("missile 0 enable (ENAM0) in COLUP0"),
        },
        PixelStrip::Colors {
            label: "m1".to_owned(),
            cells: vec![state.missile1.then_some(state.color_p1)],
            help: Some("missile 1 enable (ENAM1) in COLUP1"),
        },
        PixelStrip::Colors {
            label: "bl".to_owned(),
            cells: vec![state.ball.then_some(state.color_pf)],
            help: Some("ball enable (ENABL) in COLUPF"),
        },
    ])
}

/// The six objects the TIA draws and tests for collision, in the matrix's
/// display order. Positions index the collision pairs below.
const COLLISION_OBJECTS: [&str; 6] = ["p0", "p1", "m0", "m1", "bl", "pf"];

/// The 15 TIA collision latches as one symmetric relation over the six drawn
/// objects: each unordered pair holds when the two overlap a pixel, and stays
/// latched until CXCLR. Each entry maps a pair of [`COLLISION_OBJECTS`] positions
/// to its CXxx latch register and D7/D6 bit (per the TIA's `latch_collisions`),
/// with the source register and bit in the help.
fn tia_collision_block(state: &VcsInspectState) -> inspect::SectionBlock {
    use inspect::{PairCell, PairMatrix, SectionBlock};

    // (object a, object b, CXxx register index, D7/D6 bit, help). Object indices
    // are into COLLISION_OBJECTS: p0=0 p1=1 m0=2 m1=3 bl=4 pf=5.
    const PAIRS: [(usize, usize, usize, u8, &str); 15] = [
        (2, 1, 0, 0x80, "missile 0 vs player 1 (CXM0P D7)"),
        (2, 0, 0, 0x40, "missile 0 vs player 0 (CXM0P D6)"),
        (3, 0, 1, 0x80, "missile 1 vs player 0 (CXM1P D7)"),
        (3, 1, 1, 0x40, "missile 1 vs player 1 (CXM1P D6)"),
        (0, 5, 2, 0x80, "player 0 vs playfield (CXP0FB D7)"),
        (0, 4, 2, 0x40, "player 0 vs ball (CXP0FB D6)"),
        (1, 5, 3, 0x80, "player 1 vs playfield (CXP1FB D7)"),
        (1, 4, 3, 0x40, "player 1 vs ball (CXP1FB D6)"),
        (2, 5, 4, 0x80, "missile 0 vs playfield (CXM0FB D7)"),
        (2, 4, 4, 0x40, "missile 0 vs ball (CXM0FB D6)"),
        (3, 5, 5, 0x80, "missile 1 vs playfield (CXM1FB D7)"),
        (3, 4, 5, 0x40, "missile 1 vs ball (CXM1FB D6)"),
        (4, 5, 6, 0x80, "ball vs playfield (CXBLPF D7)"),
        (0, 1, 7, 0x80, "player 0 vs player 1 (CXPPMM D7)"),
        (2, 3, 7, 0x40, "missile 0 vs missile 1 (CXPPMM D6)"),
    ];

    let mut cells: Vec<PairCell> = (0..PairMatrix::pair_count(COLLISION_OBJECTS.len()))
        .map(|_| PairCell {
            set: false,
            help: None,
        })
        .collect();
    for &(a, b, register, bit, help) in &PAIRS {
        cells[inspect::pair_index(a, b)] = PairCell {
            set: state.collisions[register] & bit != 0,
            help: Some(help),
        };
    }
    SectionBlock::Relations(PairMatrix::new(&COLLISION_OBJECTS, cells))
}

fn tia_section(state: &VcsInspectState) -> inspect::Section {
    use inspect::{Row, SectionBlock, Sweep, SweepZone, Tone};

    // The colour clock runs 0..228: HBLANK then the 160 visible columns. The
    // field's line count is emergent from VSYNC and varies by kernel and TV
    // standard, so `line` stays a plain value rather than a fixed-period sweep.
    let beam = Sweep::new(
        "beam",
        state.beam as u32,
        crate::tia::CLOCKS_PER_LINE as u32,
    )
    .zones(vec![
        SweepZone {
            name: "hblank",
            end: crate::tia::HBLANK_CLOCKS as u32,
            tone: Tone::Idle,
        },
        SweepZone {
            name: "visible",
            end: crate::tia::CLOCKS_PER_LINE as u32,
            tone: Tone::Rendering,
        },
    ])
    .help("colour clock within the line — 0..67 hblank, 68..227 visible");

    inspect::Section {
        name: "TIA",
        summary: format!("beam {} · line {}", state.beam, state.scanline),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Sweeps(vec![beam]),
            SectionBlock::Rows(vec![
                Row::value("line", state.scanline.to_string()).help("scanline within the field"),
                Row::flag("mirror", state.pf_mirrored)
                    .help("playfield reflected on the right half (CTRLPF bit 0)"),
            ]),
            SectionBlock::Rule,
            tia_graphics_block(state),
            SectionBlock::Rule,
            tia_collision_block(state),
        ],
    }
}

/// The TIA integrates audio, but its two AUDx channels are a distinct
/// functional block, so they read clearer in their own section than appended to
/// the already-dense TIA section (beam sweep, graphics strips, collision
/// matrix). Each channel shows AUDC/AUDF/AUDV with an audibility pip.
fn audio_section(state: &VcsInspectState) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let channel = |i: usize, on_help, audc_help, audf_help, audv_help| {
        SectionBlock::Rows(vec![
            // A channel is audible only while its volume is non-zero; AUDC/AUDF
            // still divide, but the DAC drives nothing at zero AUDV.
            Row::flag(if i == 0 { "ch0" } else { "ch1" }, state.audv[i] > 0).help(on_help),
            Row::value(
                if i == 0 { "audc0" } else { "audc1" },
                format!("{:02X}", state.audc[i]),
            )
            .help(audc_help),
            Row::value(
                if i == 0 { "audf0" } else { "audf1" },
                format!("{:02X}", state.audf[i]),
            )
            .help(audf_help),
            Row::value(
                if i == 0 { "audv0" } else { "audv1" },
                format!("{:02X}", state.audv[i]),
            )
            .help(audv_help),
        ])
    };

    inspect::Section {
        name: "Audio",
        summary: format!("v{:X} v{:X}", state.audv[0], state.audv[1]),
        active: None,
        detail: None,
        blocks: vec![
            channel(
                0,
                "channel 0 audible — AUDV0 > 0",
                "waveform / tone class (AUDC0)",
                "frequency divider (AUDF0)",
                "volume (AUDV0)",
            ),
            SectionBlock::Rule,
            channel(
                1,
                "channel 1 audible — AUDV1 > 0",
                "waveform / tone class (AUDC1)",
                "frequency divider (AUDF1)",
                "volume (AUDV1)",
            ),
        ],
    }
}

fn riot_section(state: &VcsInspectState) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    inspect::Section {
        name: "RIOT",
        summary: format!("timer {:02X}", state.timer),
        active: None,
        detail: None,
        blocks: vec![SectionBlock::Rows(vec![
            Row::value("timer", format!("{:02X}", state.timer)).help("RIOT interval timer (INTIM)"),
            Row::flag("underflow", state.timer_underflowed)
                .help("timer underflowed since last read"),
            Row::value("swcha", format!("{:02X}", state.swcha)).help("controller port A (SWCHA)"),
            Row::value("swchb", format!("{:02X}", state.swchb)).help("console switches (SWCHB)"),
        ])],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CartType;

    fn cartridge_rows(section: &inspect::Section) -> Vec<String> {
        section
            .blocks
            .iter()
            .flat_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows.iter().map(|r| r.label.clone()).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn cartridge_section_shows_dpc_fetchers() {
        let cart = crate::cartridge::Cartridge::load(
            &vec![0u8; 0x2900],
            Some(CartType::Dpc),
            3_579_545.0,
            crate::cartridge::DumpFit::Exact,
        )
        .unwrap();
        let state = VcsInspectState {
            cartridge: cart.inspect(),
            ..Default::default()
        };
        let sections = vcs_sidebar_sections(&state);
        let section = sections
            .iter()
            .find(|s| s.name == "Cartridge")
            .expect("cartridge section");
        assert_eq!(section.summary, "DPC — Pitfall II (DPC)");
        let labels = cartridge_rows(section);
        for i in 0..8 {
            assert!(
                labels.iter().any(|l| *l == format!("df{i}")),
                "missing df{i}"
            );
        }
        assert!(labels.iter().any(|l| l == "rng"));
    }

    #[test]
    fn cartridge_section_shows_bank_for_banked_boards() {
        let cart = crate::cartridge::Cartridge::load(
            &vec![0u8; 0x2000],
            Some(CartType::Atari8K),
            3_579_545.0,
            crate::cartridge::DumpFit::Exact,
        )
        .unwrap();
        let state = VcsInspectState {
            cartridge: cart.inspect(),
            ..Default::default()
        };
        let section = vcs_sidebar_sections(&state)
            .into_iter()
            .find(|s| s.name == "Cartridge")
            .expect("cartridge section");
        assert!(cartridge_rows(&section).iter().any(|l| l == "bank"));
    }

    #[test]
    fn player_pattern_bit_order() {
        // Bit 7 draws leftmost with no reflect; bit 0 leads when reflected.
        assert_eq!(
            player_pattern_bits(0b1000_0001, false),
            [true, false, false, false, false, false, false, true],
        );
        assert_eq!(
            player_pattern_bits(0b1000_0001, true),
            [true, false, false, false, false, false, false, true],
        );
        assert_eq!(
            player_pattern_bits(0b1100_0000, false),
            [true, true, false, false, false, false, false, false],
        );
        assert_eq!(
            player_pattern_bits(0b1100_0000, true),
            [false, false, false, false, false, false, true, true],
        );
    }

    #[test]
    fn playfield_20_cell_pattern() {
        // PF0 bit 4 is cell 0; PF1 bit 7 is cell 4; PF2 bit 0 is cell 12.
        let cells = playfield_cells(0x10, 0x80, 0x01);
        assert!(cells[0]);
        assert!(cells[4]);
        assert!(cells[12]);
        assert_eq!(cells.iter().filter(|&&b| b).count(), 3);
        // All-clear is 20 empty cells; PF0's low nibble never contributes.
        assert_eq!(playfield_cells(0x0F, 0, 0), [false; 20]);
    }

    #[test]
    fn collision_matrix_shape_and_bit_mapping() {
        use inspect::{PairMatrix, SectionBlock};

        let mut state = VcsInspectState::default();
        // CXP0FB D7 is player 0 vs playfield: register index 2, bit 0x80.
        state.collisions[2] = 0x80;
        let SectionBlock::Relations(matrix) = tia_collision_block(&state) else {
            panic!("expected a Relations block");
        };

        assert_eq!(matrix.entities, COLLISION_OBJECTS);
        assert_eq!(matrix.cells.len(), PairMatrix::pair_count(6));
        // Every latch carries its source register/bit as help.
        assert!(matrix.cells.iter().all(|cell| cell.help.is_some()));
        // Only the p0 (0) / pf (5) pair is set.
        assert!(matrix.cell(0, 5).set);
        assert_eq!(matrix.cells.iter().filter(|cell| cell.set).count(), 1);
    }

    #[test]
    fn audio_section_reports_registers_and_audibility() {
        let state = VcsInspectState {
            audc: [0x0C, 0x00],
            audf: [0x1F, 0x00],
            audv: [0x0A, 0x00],
            ..Default::default()
        };
        let section = audio_section(&state);
        assert_eq!(section.name, "Audio");

        let rows: Vec<&inspect::Row> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        // Channel 0's AUDC/AUDV bytes render as hex.
        assert_eq!(
            rows.iter().find(|r| r.label == "audc0").map(|r| &r.value),
            Some(&"0C".to_string())
        );
        // The pip tracks volume > 0: channel 0 audible, channel 1 silent.
        assert_eq!(
            rows.iter()
                .find(|r| r.label == "ch0")
                .and_then(|r| r.active),
            Some(true)
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.label == "ch1")
                .and_then(|r| r.active),
            Some(false)
        );
    }
}
