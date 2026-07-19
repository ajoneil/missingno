//! The Game Boy Color model's implementation of the system seam's display and
//! debugger hooks: colour screen framing, the CGB register view the sidebar
//! draws, and the per-vblank snapshot that carries it.

use std::any::Any;
use std::sync::Arc;

use rgb::RGB8;

use missingno_core::cdl::CdlWindow;
use missingno_core::graphics::{GraphicsView, MapEntry, NamedPalette, PaletteSet, TileMap};
use missingno_core::inspect;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{DebugView, InspectSnapshot};
use missingno_core::video::{Frame, RgbaFrame};

use missingno_gb::Console;
use missingno_gb::debugger::graphics as gb_graphics;
use missingno_gb::debugger::inspection::{
    self as parts, AudioView, ColorSnapshot, CpuSource, GbSnapshot, PpuSource,
};
use missingno_gb::frame::NATIVE_SIZE;
use missingno_gb::ppu::memory::VramView;
use missingno_gb::ppu::types::palette::{Palette, PaletteIndex, PaletteMap};
use missingno_gb::ppu::types::tiles::TileMapId;
use missingno_gb::system::ConsoleUi;

use crate::screen::Color555;
use crate::{BgAttribute, Cgb, GameBoyColor, VramDmaStatus};

/// The 8 corrected display palettes of one CGB palette RAM.
pub fn cram_palettes(color: impl Fn(u8, u8) -> Color555) -> [Palette; 8] {
    std::array::from_fn(|palette| {
        Palette::new(std::array::from_fn(|index| {
            color(palette as u8, index as u8).to_corrected_rgb8()
        }))
    })
}

/// One CRAM bank's eight palettes as named resolved-colour palettes.
fn cram_named(prefix: &str, palettes: &[Palette; 8]) -> Vec<NamedPalette> {
    palettes
        .iter()
        .enumerate()
        .map(|(index, palette)| NamedPalette {
            label: format!("{prefix}{index}"),
            colors: (0..4).map(|c| palette.color(PaletteIndex(c))).collect(),
        })
        .collect()
}

/// The CGB graphics view: two VRAM banks as core-owned CRAM-palette atlases, the
/// two tile maps with each cell's bank-1 attribute (palette, tile bank, flips,
/// BG-over-OBJ priority), and the OAM object table with per-entry CGB
/// attributes. Composes its own two-bank view over the shared Game Boy decode
/// helpers rather than layering onto the DMG builder.
pub fn cgb_graphics_view(
    ppu: &dyn PpuSource,
    vram: &dyn VramView,
    background: &[Palette; 8],
    objects: &[Palette; 8],
) -> GraphicsView {
    let mut owned = cram_named("BG", background);
    owned.extend(cram_named("OBP", objects));

    let atlases = vec![
        gb_graphics::decode_bank_atlas(
            vram.bank(0),
            "VRAM bank 0".into(),
            PaletteSet::Owned(owned.clone()),
        ),
        gb_graphics::decode_bank_atlas(
            vram.bank(1),
            "VRAM bank 1".into(),
            PaletteSet::Owned(owned),
        ),
    ];

    let mode = ppu.control().tile_address_mode();
    let maps = [TileMapId(0), TileMapId(1)]
        .into_iter()
        .map(|id| {
            let tiles = vram.bank(0).tile_map(id);
            let attributes = vram.bank(1).tile_map(id);
            let entries = (0..32u16)
                .flat_map(|row| (0..32u16).map(move |column| (column, row)))
                .map(|(column, row)| {
                    let raw = tiles.get_tile(column as u8, row as u8);
                    let attribute = BgAttribute(attributes.get_tile(column as u8, row as u8).0);
                    MapEntry {
                        tile: gb_graphics::resolved_atlas_index(mode, raw),
                        palette: Some(attribute.palette()),
                        atlas: Some(attribute.tile_bank()),
                        flip_x: attribute.flip_x(),
                        flip_y: attribute.flip_y(),
                        priority: attribute.priority(),
                    }
                })
                .collect();
            TileMap {
                label: format!("Tile Map {}", id.0),
                columns: 32,
                rows: 32,
                atlas: 0,
                entries,
                viewports: gb_graphics::map_viewports(ppu, id),
            }
        })
        .collect();

    GraphicsView {
        atlases,
        maps,
        objects: Some(gb_graphics::object_table(ppu, true)),
    }
}

/// The CGB-only register state the sidebar draws — absent on DMG. Plain data,
/// read live when paused or copied into the snapshot while the core runs.
#[derive(Clone)]
pub struct CgbView {
    /// KEY1 speed bit: running at double speed.
    pub double_speed: bool,
    /// VBK ($FF4F) bank select.
    pub vram_bank: u8,
    /// Effective SVBK ($FF70) work-RAM bank.
    pub wram_bank: u8,
    /// OPRI ($FF6C) object-priority register.
    pub opri: u8,
    /// BCPS ($FF68) background palette index.
    pub bcps: u8,
    /// OCPS ($FF6A) object palette index.
    pub ocps: u8,
    /// VRAM-DMA (HDMA/GDMA) engine state.
    pub vram_dma: VramDmaStatus,
    /// DMG cartridge in CGB DMG-compatibility mode: the pixel FIFOs index the
    /// boot palette through BGP/OBP rather than directly.
    pub dmg_compat: bool,
    /// Raw 15-bit CRAM words per palette entry, as the hardware holds them.
    pub bg_raws: [[u16; 4]; 8],
    pub obj_raws: [[u16; 4]; 8],
}

impl CgbView {
    pub fn capture(console: &GameBoyColor) -> Self {
        let model = console.model();
        let ppu = console.ppu();
        let (bcps, ocps) = ppu.model().palette_index_registers();
        Self {
            double_speed: model.double_speed(),
            vram_bank: console.vram().selected_bank(),
            wram_bank: model.wram_bank(),
            opri: ppu.read_object_priority(),
            bcps,
            ocps,
            vram_dma: model.vram_dma_status(),
            dmg_compat: ppu.model().dmg_compat(),
            bg_raws: cram_raws(|palette, index| ppu.model().bg_color(palette, index)),
            obj_raws: cram_raws(|palette, index| ppu.model().obj_color(palette, index)),
        }
    }
}

/// The raw 15-bit words of one CGB palette RAM.
fn cram_raws(color: impl Fn(u8, u8) -> Color555) -> [[u16; 4]; 8] {
    std::array::from_fn(|palette| std::array::from_fn(|index| color(palette as u8, index as u8).0))
}

/// The CGB sidebar: CPU, PPU, and CRAM sections composed from the shared Game
/// Boy part-builders plus the colour console's own state, folded into the
/// hardware each field describes — KEY1 speed and WRAM banking in the CPU
/// section; VRAM banking, palette registers, and HDMA in the PPU section; the
/// resolved BG/OBJ palette RAM in its own CRAM section. Shared by the live
/// console (paused) and the running snapshot so the two agree by construction.
pub fn cgb_sidebar_sections(
    cpu: &impl CpuSource,
    ppu: &impl PpuSource,
    ints: &missingno_gb::interrupts::Registers,
    audio: &AudioView,
    view: &CgbView,
    background: &[Palette; 8],
    objects: &[Palette; 8],
) -> Vec<inspect::Section> {
    use inspect::{Row, Section, SectionBlock};

    let speed = if view.double_speed { "2x" } else { "1x" };
    let mut cpu_content = parts::cpu_blocks(cpu, ints);
    cpu_content.push(SectionBlock::Rows(vec![
        Row::value("speed", speed).help("CPU speed (KEY1) — 1x or 2x double speed"),
        Row::value("svbk", view.wram_bank.to_string()).help("work-RAM bank (SVBK)"),
    ]));

    let ppu_content = vec![
        parts::ppu_position_block(ppu),
        parts::ppu_status_block(ppu),
        SectionBlock::Rule,
        parts::ppu_background_block(ppu),
        SectionBlock::Rule,
        parts::ppu_window_block(ppu),
        SectionBlock::Rule,
        parts::ppu_sprites_block(ppu),
        SectionBlock::Rows(vec![
            Row::value("vbk", view.vram_bank.to_string()).help("VRAM bank (VBK)"),
            Row::value("opri", format!("{:02X}", view.opri)).help("object priority mode (OPRI)"),
            Row::value("bcps", format!("{:02X}", view.bcps))
                .help("background palette index (BCPS)"),
            Row::value("ocps", format!("{:02X}", view.ocps)).help("object palette index (OCPS)"),
            Row::value("hdma", hdma_status(view.vram_dma)).help("VRAM DMA (HDMA/GDMA) status"),
        ]),
        SectionBlock::Rule,
        cgb_fifo_block(ppu, background, objects, view),
    ];

    let cram_content = vec![
        SectionBlock::Swatches(cram_swatches("bg", background, &view.bg_raws)),
        SectionBlock::Swatches(cram_swatches("obj", objects, &view.obj_raws)),
    ];

    vec![
        Section {
            name: "CPU",
            summary: parts::cpu_summary(cpu),
            active: Some(!cpu.halted()),
            detail: None,
            blocks: cpu_content,
        },
        Section {
            name: "PPU",
            summary: parts::ppu_summary(ppu),
            active: Some(ppu.control().video_enabled()),
            detail: Some(parts::ppu_detail(ppu)),
            blocks: ppu_content,
        },
        Section {
            name: "CRAM",
            summary: format!("bcps {:02X} · ocps {:02X}", view.bcps, view.ocps),
            active: None,
            detail: None,
            blocks: cram_content,
        },
        parts::apu_section(audio),
    ]
}

/// The two pixel FIFOs as CGB colour strips: each cell resolves through palette
/// RAM the core owns. In full-CGB mode a background cell indexes its tile's BG
/// palette by the raw 2-bit colour, an object cell its OBP0-7 palette; a
/// transparent object pixel (colour 0) and an off pipeline render as unlit
/// cells. In DMG-compatibility mode the colour first maps through BGP/OBP to a
/// shade that indexes the boot palette. Snapshots taken at vblank catch the
/// FIFOs empty; the strips fill when paused mid-scanline.
fn cgb_fifo_block(
    ppu: &impl PpuSource,
    background: &[Palette; 8],
    objects: &[Palette; 8],
    view: &CgbView,
) -> inspect::SectionBlock {
    use inspect::PixelStrip;

    let bg_cells: Vec<Option<RGB8>> = match ppu.bg_fifo() {
        Some(cells) => cells
            .iter()
            .map(|c| {
                let (palette, index) = if view.dmg_compat {
                    (0, PaletteMap(ppu.bgp()).map(PaletteIndex(c.color)).0)
                } else {
                    (c.palette, c.color)
                };
                Some(background[palette as usize].color(PaletteIndex(index)))
            })
            .collect(),
        None => vec![None; 8],
    };

    let obj_cells: Vec<Option<RGB8>> = match ppu.obj_fifo() {
        Some(cells) => cells
            .iter()
            .map(|c| {
                if c.color == 0 {
                    return None;
                }
                let index = if view.dmg_compat {
                    let obp = if c.palette == 0 {
                        ppu.obp0()
                    } else {
                        ppu.obp1()
                    };
                    PaletteMap(obp).map(PaletteIndex(c.color)).0
                } else {
                    c.color
                };
                Some(objects[c.palette as usize].color(PaletteIndex(index)))
            })
            .collect(),
        None => vec![None; 8],
    };

    inspect::SectionBlock::Pixels(vec![
        PixelStrip::Colors {
            label: "bg fifo".to_owned(),
            cells: bg_cells,
            help: Some("background pixel FIFO — colour through BG palette RAM; next pixel at left"),
        },
        PixelStrip::Colors {
            label: "obj fifo".to_owned(),
            cells: obj_cells,
            help: Some(
                "object pixel FIFO — colour through OBJ palette RAM; colour 0 transparent, discarded before palette",
            ),
        },
    ])
}

/// The eight resolved palettes of one CRAM bank as swatch rows, each swatch
/// carrying its raw 15-bit CRAM word.
fn cram_swatches(
    prefix: &str,
    palettes: &[Palette; 8],
    raws: &[[u16; 4]; 8],
) -> Vec<inspect::SwatchRow> {
    use missingno_gb::ppu::types::palette::PaletteIndex;

    palettes
        .iter()
        .zip(raws)
        .enumerate()
        .map(|(index, (palette, words))| inspect::SwatchRow::Colors {
            label: format!("{prefix}{index}"),
            colors: (0..4)
                .map(|i| inspect::ColorSwatch {
                    color: palette.color(PaletteIndex(i)),
                    raw: Some(words[i as usize]),
                })
                .collect(),
        })
        .collect()
}

fn hdma_status(status: VramDmaStatus) -> String {
    match status {
        VramDmaStatus::Idle => "idle".to_owned(),
        VramDmaStatus::General { remaining } => format!("gdma {remaining}B"),
        VramDmaStatus::HBlank {
            remaining,
            source,
            dest,
        } => format!("hdma {remaining}B {source:04X}\u{2192}{dest:04X}"),
    }
}

/// A per-vblank snapshot: the model-shared state plus the CGB register view.
pub struct CgbSnapshot {
    pub base: GbSnapshot,
    pub cgb: CgbView,
}

impl InspectSnapshot for CgbSnapshot {
    fn frame(&self) -> u64 {
        self.base.frame
    }
    fn family_state(&self) -> &dyn Any {
        self
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        self.base.register_groups()
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        let ColorSnapshot::Cgb {
            background,
            objects,
        } = &self.base.colors
        else {
            return self.base.sidebar_sections();
        };
        cgb_sidebar_sections(
            &self.base.cpu,
            &self.base.ppu,
            &self.base.interrupts,
            &self.base.audio,
            &self.cgb,
            background,
            objects,
        )
    }
    fn memory_window(&self) -> Option<&inspect::MemoryWindow> {
        self.base.memory_window()
    }
    fn pc(&self) -> Option<u32> {
        self.base.pc()
    }
    fn symbols(&self) -> Option<&missingno_core::symbols::SymbolTable> {
        self.base.symbols()
    }
    fn cdl_window(&self) -> Option<&missingno_core::cdl::CdlWindow> {
        self.base.cdl_window()
    }
    fn bank_for(&self, address: u32) -> Option<u16> {
        self.base.bank_for(address)
    }
    fn instruction_set(&self) -> Option<&dyn missingno_core::isa::InstructionSet> {
        self.base.instruction_set()
    }
    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.base.channel_waves()
    }
    fn graphics(&self) -> Option<GraphicsView> {
        self.base.graphics()
    }
}

impl ConsoleUi for Cgb {
    const MONOCHROME_PALETTE: bool = false;

    fn screen_display(console: &Console<Self>, new_screen: Option<Self::Screen>) -> Option<Frame> {
        if !console.ppu().control().video_enabled() {
            Some(Frame::Rgba(RgbaFrame::blank(NATIVE_SIZE.0, NATIVE_SIZE.1)))
        } else {
            new_screen.map(|screen| {
                Frame::Rgba(RgbaFrame {
                    width: NATIVE_SIZE.0,
                    height: NATIVE_SIZE.1,
                    pixels: screen.to_corrected_rgba().into(),
                    pixel_aspect: 1.0,
                })
            })
        }
    }

    fn snapshot(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> DebugView {
        let ppu = console.ppu().model();
        let colors = ColorSnapshot::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        };
        let graphics = console
            .graphics_capture()
            .then(|| Self::graphics_view(console));
        let base = GbSnapshot::capture(console, colors, frame, symbols, cdl, graphics);
        Box::new(CgbSnapshot {
            cgb: CgbView::capture(console),
            base,
        })
    }

    fn graphics_view(console: &Console<Self>) -> GraphicsView {
        let ppu = console.ppu().model();
        let background = cram_palettes(|palette, index| ppu.bg_color(palette, index));
        let objects = cram_palettes(|palette, index| ppu.obj_color(palette, index));
        cgb_graphics_view(console.ppu(), console.vram(), &background, &objects)
    }

    fn sidebar_sections(console: &Console<Self>) -> Vec<inspect::Section> {
        let ppu = console.ppu().model();
        let background = cram_palettes(|palette, index| ppu.bg_color(palette, index));
        let objects = cram_palettes(|palette, index| ppu.obj_color(palette, index));
        cgb_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &AudioView::capture(console.audio()),
            &CgbView::capture(console),
            &background,
            &objects,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_gb::cartridge::Cartridge;
    use missingno_gb::debugger::Debugger;

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger = Debugger::new(GameBoyColor::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
        let console = debugger.game_boy();

        let live = Cgb::sidebar_sections(console);
        let snapshot = Cgb::snapshot(
            console,
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.sidebar_sections())
        );
    }

    #[test]
    fn sidebar_carries_the_shared_apu_section() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger = Debugger::new(GameBoyColor::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
        let sections = Cgb::sidebar_sections(debugger.game_boy());
        assert!(sections.iter().any(|section| section.name == "APU"));
    }

    #[test]
    fn cram_swatches_carry_the_raw_words() {
        let debugger = stepped_cgb();
        let sections = Cgb::sidebar_sections(debugger.game_boy());
        let cram = sections
            .iter()
            .find(|section| section.name == "CRAM")
            .expect("CRAM section");
        let raws: Vec<u16> = cram
            .blocks
            .iter()
            .filter_map(|block| match block {
                inspect::SectionBlock::Swatches(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .filter_map(|row| match row {
                inspect::SwatchRow::Colors { colors, .. } => Some(colors),
                _ => None,
            })
            .flatten()
            .filter_map(|swatch| swatch.raw)
            .collect();
        // 2 banks × 8 palettes × 4 colours, every swatch carrying its word.
        assert_eq!(raws.len(), 64);
        // The boot fade seeds BG palettes white ($7FFF).
        assert_eq!(raws[0], 0x7FFF);
    }

    fn stepped_cgb() -> Debugger<Cgb> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger = Debugger::new(GameBoyColor::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
        debugger
    }

    #[test]
    fn snapshot_graphics_matches_live_and_is_gated() {
        let mut debugger = stepped_cgb();
        // Disabled: the snapshot carries no graphics.
        let off = Cgb::snapshot(
            debugger.game_boy(),
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert!(off.graphics().is_none());

        // Enabled: the running snapshot equals the live (paused) view.
        debugger.game_boy_mut().set_graphics_capture(true);
        let console = debugger.game_boy();
        let live = Cgb::graphics_view(console);
        let on = Cgb::snapshot(
            console,
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(on.graphics(), Some(live.clone()));
        // Two banks, both core-owned CRAM palettes; maps carry per-cell palette.
        assert_eq!(live.atlases.len(), 2);
        assert!(
            live.atlases
                .iter()
                .all(|a| matches!(a.palettes, PaletteSet::Owned(_)))
        );
        // Each bank carries the three tile-data blocks, covering its atlas.
        assert!(live.atlases.iter().all(|a| {
            a.regions_valid()
                && a.regions
                    .iter()
                    .map(|r| r.label)
                    .eq(["Block 0", "Block 1", "Block 2"])
        }));
        assert!(live.maps.iter().all(|m| {
            m.entries
                .iter()
                .all(|e| e.palette.is_some() && e.atlas.is_some())
        }));
    }

    #[test]
    fn cgb_attribute_decodes_to_map_entry_fields() {
        use missingno_gb::ppu::memory::{VramBank, VramView};

        struct TwoBank {
            banks: [VramBank; 2],
        }
        impl VramView for TwoBank {
            fn bank(&self, bank: u8) -> &VramBank {
                &self.banks[bank as usize]
            }
        }

        // Bank 0: tile index 5 at map-0 cell (0,0) — flat VRAM offset 0x1800.
        let mut b0 = vec![0u8; 0x2000];
        b0[0x1800] = 5;
        // Bank 1: attribute 0x2B at the same cell — palette 3, tile bank 1,
        // X-flip set, Y-flip clear, priority clear.
        let mut b1 = vec![0u8; 0x2000];
        b1[0x1800] = 0x2B;
        let vram = TwoBank {
            banks: [VramBank::from_bytes(&b0), VramBank::from_bytes(&b1)],
        };

        let debugger = stepped_cgb();
        let ppu = debugger.game_boy().ppu();
        let palettes = cram_palettes(|_, _| Color555(0));
        let view = cgb_graphics_view(ppu, &vram, &palettes, &palettes);

        let entry = view.maps[0].entry(0, 0).expect("cell present");
        assert_eq!(entry.tile, 5); // Block0Block1 identity for index 5.
        assert_eq!(entry.palette, Some(3));
        assert_eq!(entry.atlas, Some(1));
        assert!(entry.flip_x);
        assert!(!entry.flip_y);
        assert!(!entry.priority);
    }
}
