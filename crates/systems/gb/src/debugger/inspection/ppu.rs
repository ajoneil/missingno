use missingno_core::inspect;

use crate::ppu::{
    BgFifoCell, ObjFifoCell, Ppu, Register,
    model::PpuModel,
    rendering::Mode,
    types::{
        control::Control,
        palette::{Palette, PaletteIndex, PaletteMap},
        sprites::{Sprite, SpriteId, SpriteSize},
        tiles::TileAddressMode,
    },
};

/// The 40 hardware sprites.
const SPRITE_COUNT: usize = 40;

/// The PPU register/OAM state the tile-map, sprite, and PPU-sidebar panes draw.
pub trait PpuSource {
    fn control(&self) -> Control;
    fn mode(&self) -> Mode;
    /// The raw STAT byte — its mode bits plus the LYC-coincidence flag and the
    /// mode/LYC interrupt-enable bits the decoded rows don't otherwise carry.
    fn stat(&self) -> u8;
    fn ly(&self) -> u8;
    fn lx(&self) -> u8;
    /// The LY-compare register (LYC) driving the STAT coincidence flag.
    fn lyc(&self) -> u8;
    fn scx(&self) -> u8;
    fn scy(&self) -> u8;
    fn wx(&self) -> u8;
    fn wy(&self) -> u8;
    fn bgp(&self) -> u8;
    fn obp0(&self) -> u8;
    fn obp1(&self) -> u8;
    fn sprite(&self, id: SpriteId) -> Sprite;
    /// The background pixel-shifter's 8 stages, or `None` with the LCD off.
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]>;
    /// The object FIFO's 8 stages, or `None` with the LCD off.
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]>;
    /// The OAM-scan progress entry during mode 2, or `None` outside it (or with
    /// the LCD off).
    fn scan_counter(&self) -> Option<u8>;
}

impl<P: PpuModel> PpuSource for Ppu<P> {
    fn control(&self) -> Control {
        Ppu::control(self)
    }
    fn mode(&self) -> Mode {
        Ppu::mode(self)
    }
    fn stat(&self) -> u8 {
        self.read_register(Register::Status)
    }
    fn ly(&self) -> u8 {
        self.video.ly()
    }
    fn lx(&self) -> u8 {
        Ppu::lx(self)
    }
    fn lyc(&self) -> u8 {
        self.read_register(Register::InterruptOnScanline)
    }
    fn scx(&self) -> u8 {
        self.read_register(Register::BackgroundViewportX)
    }
    fn scy(&self) -> u8 {
        self.read_register(Register::BackgroundViewportY)
    }
    fn wx(&self) -> u8 {
        self.read_register(Register::WindowX)
    }
    fn wy(&self) -> u8 {
        self.read_register(Register::WindowY)
    }
    fn bgp(&self) -> u8 {
        self.palettes().background.output()
    }
    fn obp0(&self) -> u8 {
        self.palettes().sprite0.output()
    }
    fn obp1(&self) -> u8 {
        self.palettes().sprite1.output()
    }
    fn sprite(&self, id: SpriteId) -> Sprite {
        *Ppu::sprite(self, id)
    }
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]> {
        Ppu::bg_fifo(self)
    }
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]> {
        Ppu::obj_fifo(self)
    }
    fn scan_counter(&self) -> Option<u8> {
        Ppu::scan_counter(self)
    }
}

#[derive(Clone)]
pub struct PpuView {
    control: Control,
    pub(super) mode: Mode,
    stat: u8,
    pub(super) ly: u8,
    lx: u8,
    lyc: u8,
    scx: u8,
    scy: u8,
    wx: u8,
    wy: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    sprites: [Sprite; SPRITE_COUNT],
    bg_fifo: Option<[BgFifoCell; 8]>,
    obj_fifo: Option<[ObjFifoCell; 8]>,
    scan_counter: Option<u8>,
}

impl PpuView {
    pub(super) fn capture(ppu: &impl PpuSource) -> Self {
        Self {
            control: ppu.control(),
            mode: ppu.mode(),
            stat: ppu.stat(),
            ly: ppu.ly(),
            lx: ppu.lx(),
            lyc: ppu.lyc(),
            scx: ppu.scx(),
            scy: ppu.scy(),
            wx: ppu.wx(),
            wy: ppu.wy(),
            bgp: ppu.bgp(),
            obp0: ppu.obp0(),
            obp1: ppu.obp1(),
            sprites: std::array::from_fn(|i| ppu.sprite(SpriteId(i as u8))),
            bg_fifo: ppu.bg_fifo(),
            obj_fifo: ppu.obj_fifo(),
            scan_counter: ppu.scan_counter(),
        }
    }
}

impl PpuSource for PpuView {
    fn control(&self) -> Control {
        self.control
    }
    fn mode(&self) -> Mode {
        self.mode
    }
    fn stat(&self) -> u8 {
        self.stat
    }
    fn ly(&self) -> u8 {
        self.ly
    }
    fn lx(&self) -> u8 {
        self.lx
    }
    fn lyc(&self) -> u8 {
        self.lyc
    }
    fn scx(&self) -> u8 {
        self.scx
    }
    fn scy(&self) -> u8 {
        self.scy
    }
    fn wx(&self) -> u8 {
        self.wx
    }
    fn wy(&self) -> u8 {
        self.wy
    }
    fn bgp(&self) -> u8 {
        self.bgp
    }
    fn obp0(&self) -> u8 {
        self.obp0
    }
    fn obp1(&self) -> u8 {
        self.obp1
    }
    fn sprite(&self, id: SpriteId) -> Sprite {
        self.sprites[id.0 as usize]
    }
    fn bg_fifo(&self) -> Option<[BgFifoCell; 8]> {
        self.bg_fifo
    }
    fn obj_fifo(&self) -> Option<[ObjFifoCell; 8]> {
        self.obj_fifo
    }
    fn scan_counter(&self) -> Option<u8> {
        self.scan_counter
    }
}

/// The palette-independent colour data published while the core runs, so the
/// running panes can rebuild their render palettes with the live user palette
/// (which can change mid-run on DMG).
// One snapshot per vblank; boxing the CGB arrays would just add a hop.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum ColorSnapshot {
    Dmg {
        sgb: bool,
    },
    Cgb {
        background: [Palette; 8],
        objects: [Palette; 8],
    },
}

/// The accent class for a PPU mode's inline detail.
fn mode_tone(mode: Mode) -> inspect::Tone {
    match mode {
        Mode::HorizontalBlank => inspect::Tone::Idle,
        Mode::VerticalBlank => inspect::Tone::Active,
        Mode::OamScan => inspect::Tone::Scanning,
        Mode::Drawing => inspect::Tone::Rendering,
    }
}

/// The PPU section's collapsed summary.
pub fn ppu_summary(ppu: &impl PpuSource) -> String {
    format!("{} · ly {}", ppu.mode(), ppu.ly())
}

/// The accented PPU-mode detail beside the section heading.
pub fn ppu_detail(ppu: &impl PpuSource) -> inspect::Detail {
    let mode = ppu.mode();
    inspect::Detail {
        text: mode.to_string(),
        tone: mode_tone(mode),
    }
}

/// The ly/lx position sweeps: LY across the 154-line frame (144 visible lines
/// then 10 vblank lines), LX across the internal line counter (0 up to the SANU
/// line-end decode); mode boundaries within the line vary, so LX carries no
/// zones.
pub fn ppu_position_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    use inspect::{Sweep, SweepZone, Tone};

    let ly = Sweep::new("ly", ppu.ly() as u32, 154)
        .zones(vec![
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
        .help("current scanline (LY) — 0..143 visible, 144..153 vblank");
    // The LX counter resets at the SANU line-end decode (value 113).
    let lx = Sweep::new("lx", ppu.lx() as u32, 114)
        .help("dot position within the scanline (LX counter)");

    inspect::SectionBlock::Sweeps(vec![ly, lx])
}

/// The raw STAT and LYC registers: STAT carries the LYC-coincidence flag and
/// the mode/LYC interrupt-enable bits the decoded rows don't otherwise show,
/// and LYC is the compare value that drives the coincidence flag.
pub fn ppu_status_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    // The scan counter advances only in mode 2; outside it the entry is stale,
    // so the row reads "-".
    let scan = match (ppu.mode(), ppu.scan_counter()) {
        (Mode::OamScan, Some(entry)) => entry.to_string(),
        _ => "-".to_string(),
    };
    inspect::SectionBlock::Rows(vec![
        inspect::Row::value("stat", format!("{:02X}", ppu.stat()))
            .help("LCD status (STAT) — mode, LYC coincidence, and mode/LYC interrupt enables"),
        inspect::Row::value("lyc", format!("{:02X}", ppu.lyc()))
            .help("LY compare (LYC) — matches LY to raise the STAT coincidence flag"),
        inspect::Row::value("scan", scan).help("OAM scan entry (mode 2)"),
    ])
}

/// The background enable/map/tile and scroll rows.
pub fn ppu_background_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("bg", control.background_and_window_enabled())
            .help("background & window enable (LCDC bit 0)"),
        inspect::Row::value("map", tile_map_addr(control.background_tile_map().0))
            .help("background tile-map base address"),
        inspect::Row::value("tile", tile_addr(control.tile_address_mode()))
            .help("tile-data addressing mode (LCDC bit 4)"),
        inspect::Row::value("scx", format!("{:02X}", ppu.scx())).help("background scroll X (SCX)"),
        inspect::Row::value("scy", format!("{:02X}", ppu.scy())).help("background scroll Y (SCY)"),
    ])
}

/// The window enable/map and position rows.
pub fn ppu_window_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("win", control.window_enabled()).help("window enable (LCDC bit 5)"),
        inspect::Row::value("map", tile_map_addr(control.window_tile_map().0))
            .help("window tile-map base address"),
        inspect::Row::value("wx", format!("{:02X}", ppu.wx())).help("window X position (WX)"),
        inspect::Row::value("wy", format!("{:02X}", ppu.wy())).help("window Y position (WY)"),
    ])
}

/// The sprite enable and size rows.
pub fn ppu_sprites_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    let control = ppu.control();
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("sprites", control.sprites_enabled())
            .help("object (sprite) enable (LCDC bit 1)"),
        inspect::Row::value(
            "size",
            match control.sprite_size() {
                SpriteSize::Single => "8×8",
                SpriteSize::Double => "8×16",
            },
        )
        .help("object size (LCDC bit 2)"),
    ])
}

/// The DMG background palette (BGP) as a packed shade-swatch row.
pub fn dmg_background_swatches(ppu: &impl PpuSource) -> inspect::SectionBlock {
    inspect::SectionBlock::Swatches(vec![inspect::SwatchRow::Shades {
        label: "bgp",
        packed: ppu.bgp(),
    }])
}

/// The two pixel FIFOs as DMG shade strips: each cell is the 2-bit colour
/// mapped through its palette register (BGP for background, the pixel's
/// OBP0/OBP1 select for objects) to a shade the frontend then resolves through
/// the user palette. A transparent object pixel (colour 0) and an off pipeline
/// render as unlit cells. Snapshots taken at vblank catch the FIFOs empty; the
/// strips fill when paused mid-scanline.
pub fn dmg_fifo_block(ppu: &impl PpuSource) -> inspect::SectionBlock {
    use inspect::PixelStrip;

    inspect::SectionBlock::Pixels(vec![
        PixelStrip::Shades {
            label: "bg fifo",
            cells: dmg_bg_strip(ppu.bg_fifo(), ppu.bgp()),
            help: Some("background pixel FIFO — colour through BGP; next pixel at left"),
        },
        PixelStrip::Shades {
            label: "obj fifo",
            cells: dmg_obj_strip(ppu.obj_fifo(), ppu.obp0(), ppu.obp1()),
            help: Some(
                "object pixel FIFO — colour through OBP0/OBP1; colour 0 transparent, discarded before palette",
            ),
        },
    ])
}

/// Each background cell's colour mapped through BGP to a shade — every cell is a
/// real colour (colour 0 is an opaque BG shade); an off pipeline is eight unlit
/// cells.
fn dmg_bg_strip(fifo: Option<[BgFifoCell; 8]>, bgp: u8) -> Vec<Option<u8>> {
    match fifo {
        Some(cells) => cells
            .iter()
            .map(|c| Some(PaletteMap(bgp).map(PaletteIndex(c.color)).0))
            .collect(),
        None => vec![None; 8],
    }
}

/// Each object cell's colour mapped through its OBP0/OBP1 select to a shade;
/// colour 0 (transparent) and an off pipeline render as empty cells.
fn dmg_obj_strip(fifo: Option<[ObjFifoCell; 8]>, obp0: u8, obp1: u8) -> Vec<Option<u8>> {
    match fifo {
        Some(cells) => cells
            .iter()
            .map(|c| {
                (c.color != 0).then(|| {
                    let obp = if c.palette == 0 { obp0 } else { obp1 };
                    PaletteMap(obp).map(PaletteIndex(c.color)).0
                })
            })
            .collect(),
        None => vec![None; 8],
    }
}

/// The DMG object palettes (OBP0/OBP1) as packed shade-swatch rows.
pub fn dmg_object_swatches(ppu: &impl PpuSource) -> inspect::SectionBlock {
    inspect::SectionBlock::Swatches(vec![
        inspect::SwatchRow::Shades {
            label: "obp0",
            packed: ppu.obp0(),
        },
        inspect::SwatchRow::Shades {
            label: "obp1",
            packed: ppu.obp1(),
        },
    ])
}

fn tile_map_addr(id: u8) -> &'static str {
    if id == 0 { "9800" } else { "9C00" }
}

fn tile_addr(mode: TileAddressMode) -> &'static str {
    match mode {
        TileAddressMode::Block0Block1 => "8000",
        TileAddressMode::Block2Block1 => "8800",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspection::tests::stepped_dmg;
    use crate::debugger::inspection::{AudioView, TimersView, dmg_sidebar_sections};

    #[test]
    fn dmg_bg_strip_maps_colour_through_bgp() {
        // BGP 0b11_10_01_00: colour 0→0, 1→1, 2→2, 3→3 (identity).
        let cells = std::array::from_fn(|i| BgFifoCell {
            color: (i % 4) as u8,
            palette: 0,
        });
        let strip = dmg_bg_strip(Some(cells), 0b11_10_01_00);
        assert_eq!(strip[0], Some(0));
        assert_eq!(strip[1], Some(1));
        assert_eq!(strip[2], Some(2));
        assert_eq!(strip[3], Some(3));
        // An off pipeline is eight empty cells.
        assert_eq!(dmg_bg_strip(None, 0xE4), vec![None; 8]);
    }

    #[test]
    fn dmg_obj_strip_transparency_and_palette_select() {
        let cell = |color, palette| ObjFifoCell {
            color,
            palette,
            priority: 0,
        };
        // OBP0 identity; OBP1 = 0b00_01_10_11 maps colour 1→2, 3→0.
        let cells = [
            cell(0, 0), // transparent → empty
            cell(1, 0), // OBP0: shade 1
            cell(1, 1), // OBP1: shade 2
            cell(0, 1), // transparent → empty
            cell(2, 0), // OBP0: shade 2
            cell(3, 1), // OBP1: shade 0
            cell(0, 0),
            cell(0, 0),
        ];
        let strip = dmg_obj_strip(Some(cells), 0b11_10_01_00, 0b00_01_10_11);
        assert_eq!(strip[0], None);
        assert_eq!(strip[1], Some(1));
        assert_eq!(strip[2], Some(2));
        assert_eq!(strip[3], None);
        assert_eq!(strip[4], Some(2));
        assert_eq!(strip[5], Some(0));
        assert_eq!(dmg_obj_strip(None, 0xE4, 0xE4), vec![None; 8]);
    }

    #[test]
    fn ppu_status_block_carries_scan_row() {
        let debugger = stepped_dmg();
        let ppu = PpuView::capture(debugger.game_boy().ppu());
        let block = ppu_status_block(&ppu);
        let labels = match &block {
            inspect::SectionBlock::Rows(rows) => {
                rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>()
            }
            _ => panic!("expected rows"),
        };
        assert!(labels.iter().any(|l| l == "scan"), "missing scan row");
    }

    #[test]
    fn dmg_swatch_blocks_carry_packed_registers() {
        let debugger = stepped_dmg();
        let ppu = PpuView::capture(debugger.game_boy().ppu());

        let rows: Vec<_> = [dmg_background_swatches(&ppu), dmg_object_swatches(&ppu)]
            .into_iter()
            .flat_map(|block| match block {
                inspect::SectionBlock::Swatches(rows) => rows,
                _ => panic!("expected swatches"),
            })
            .collect();
        let expected = [
            ("bgp", ppu.bgp()),
            ("obp0", ppu.obp0()),
            ("obp1", ppu.obp1()),
        ];
        assert_eq!(rows.len(), expected.len());
        for (row, (label, packed)) in rows.iter().zip(expected) {
            match row {
                inspect::SwatchRow::Shades {
                    label: got_label,
                    packed: got_packed,
                } => {
                    assert_eq!(*got_label, label);
                    assert_eq!(*got_packed, packed);
                }
                _ => panic!("expected packed shades"),
            }
        }

        // The DMG PPU section places both swatch blocks with its registers.
        let console = debugger.game_boy();
        let audio = AudioView::capture(console.audio());
        let timers = TimersView::capture(console.timers());
        let cart = console.cartridge().inspect();
        let sections = dmg_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &timers,
            &audio,
            &cart,
        );
        let swatch_blocks = sections[1]
            .blocks
            .iter()
            .filter(|block| matches!(block, inspect::SectionBlock::Swatches(_)))
            .count();
        assert_eq!(swatch_blocks, 2);
    }
}
