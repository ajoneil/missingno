//! The Atari VCS's implementation of the system seam and its debugger
//! inspection state. One owned state struct serves both the paused view
//! (refreshed after every step) and the per-frame snapshot the running view
//! renders from.

use std::collections::BTreeSet;
use std::time::Duration;

use rgb::RGB8;

use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::state::{PixelFormat, StateRecord, SystemStateSchema};
use missingno_core::state_file::{StateFrame, StateMeta, read_state_file, write_state_file};
use missingno_core::system::{
    ConsoleSwitch, ControlId, ControlInput, DebugView, FrameOutcome, InspectSnapshot,
    RunningStatus, StateError, StepOutcome, SystemConsole, SystemDebugger,
};
use missingno_core::video::{
    self, DisplayTechnology, Frame as VideoFrame, IndexedFrame, Television,
};

use crate::state_schema::vcs_state_schema;

use crate::cartridge::CartridgeError;
use crate::console::{Frame, JoystickDirection, Vcs};
use crate::tia::{VISIBLE_CLOCKS, palette_index};
use crate::tv_standard::PIXEL_ASPECT;
use crate::{CartType, TvStandard};

/// The latching console switches, driven through control ids past the
/// paddle (id 8). Positions and defaults match the RIOT's SWCHB state.
pub const CONSOLE_SWITCHES: [ConsoleSwitch; 3] = [
    ConsoleSwitch {
        control: ControlId(9),
        label: "Left Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(10),
        label: "Right Difficulty",
        positions: ["B", "A"],
        default_high: false,
    },
    ConsoleSwitch {
        control: ControlId(11),
        label: "TV Type",
        positions: ["B•W", "Color"],
        default_high: true,
    },
];

/// Nominal frame: a full field of 228-clock lines at the colour clock — 262
/// lines (NTSC) or 312 (PAL). Kernels vary line counts; pacing uses the
/// convention so the frame rate follows the broadcast standard.
fn frame_interval(standard: TvStandard) -> Duration {
    let lines = match standard {
        TvStandard::Ntsc => 262.0,
        TvStandard::Pal | TvStandard::Secam => 312.0,
    };
    Duration::from_secs_f32(lines * 228.0 / crate::tv_standard::master_clock_hz(standard))
}

/// Frames are emergent from VSYNC; bound the search so a kernel that never
/// syncs cannot stall the emulation thread.
const FRAME_BUDGET_LINES: usize = 1000;

/// Scanlines of asserted VSYNC the television integrates before the field
/// re-anchors. The console drives VSYNC as a plain latch; this lock lives in
/// the set (off-chip) and is calibratable — reference emulators model 2 and the
/// safe kernel convention is 3, so anything shorter is swallowed.
const VSYNC_LOCK_LINES: usize = 2;

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide).
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => matches!(rom.len(), 0x800 | 0x1000),
        _ => false,
    }
}

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    // The library's metadata is authoritative; carts carry no region header and
    // the size heuristic can't always name the board, so fall back only when
    // the game-db is silent — then probe the standard from the ROM's own field
    // length. Pacing, aspect, and palette follow the standard.
    let cart = cart_type.and_then(core_cart_type);
    let region = match tv_standard {
        Some(standard) => standard,
        None => probe_tv_standard(rom, cart),
    };
    Ok(Box::new(VcsConsole {
        vcs: Vcs::new(rom, region, cart)?,
        title,
        rom_sha256: rom_fingerprint(rom),
        last_frame: blank_frame(),
        tv: Television::new(VSYNC_LOCK_LINES),
    }))
}

/// Scanlines per field that split NTSC (~262) from PAL (~312); the midpoint
/// clears the ~284–290 overlap where a handful of ROMs are genuinely ambiguous.
const NTSC_PAL_FIELD_THRESHOLD: usize = 287;

/// Detect an uncatalogued ROM's broadcast standard by counting scanlines per
/// field: a PAL field runs ~50 lines longer than NTSC. The standard only scales
/// the master clock, not the kernel's line count, so a provisional NTSC build
/// reads the field length truthfully.
fn probe_tv_standard(rom: &[u8], cart_type: Option<CartType>) -> TvStandard {
    let Ok(mut vcs) = Vcs::new(rom, TvStandard::Ntsc, cart_type) else {
        return TvStandard::Ntsc;
    };
    let mut tv = Television::<VISIBLE_CLOCKS>::new(VSYNC_LOCK_LINES);
    let mut fields = Vec::new();
    let mut lines_this_field = 0usize;
    // A few fields, bounded so a kernel that never syncs can't spin.
    for _ in 0..(FRAME_BUDGET_LINES * 8) {
        let line = vcs.step_scanline();
        lines_this_field += 1;
        if tv
            .feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            })
            .is_some()
        {
            fields.push(lines_this_field);
            lines_this_field = 0;
            if fields.len() >= 6 {
                break;
            }
        }
    }
    classify_fields(&fields)
}

/// Classify measured field lengths by their median (robust to a long startup
/// field), skipping the first warm-up field; NTSC when nothing synced.
fn classify_fields(fields: &[usize]) -> TvStandard {
    let mut steady: Vec<usize> = fields.iter().copied().skip(1).collect();
    if steady.is_empty() {
        return TvStandard::Ntsc;
    }
    steady.sort_unstable();
    if steady[steady.len() / 2] > NTSC_PAL_FIELD_THRESHOLD {
        TvStandard::Pal
    } else {
        TvStandard::Ntsc
    }
}

/// Parse a game-db board code into the core's board type; codes the core can't
/// build yet return `None`, leaving `Cartridge::load` to size-detect.
fn core_cart_type(code: &str) -> Option<CartType> {
    match code {
        "2K" => Some(CartType::Plain2K),
        "4K" => Some(CartType::Plain4K),
        "F8" => Some(CartType::F8),
        "F8SC" => Some(CartType::F8Sc),
        "F6" => Some(CartType::F6),
        "F6SC" => Some(CartType::F6Sc),
        "F4" => Some(CartType::F4),
        "F4SC" => Some(CartType::F4Sc),
        "FA" => Some(CartType::Fa),
        "FC" => Some(CartType::Fc),
        "FE" => Some(CartType::Fe),
        "E0" => Some(CartType::E0),
        "E7" => Some(CartType::E7),
        "CV" => Some(CartType::Cv),
        "UA" => Some(CartType::Ua),
        "3F" => Some(CartType::ThreeF),
        "3E" => Some(CartType::ThreeE),
        "3E+" => Some(CartType::ThreeEPlus),
        "DPC" => Some(CartType::Dpc),
        "AR" => Some(CartType::Ar),
        "F0" => Some(CartType::F0),
        "JANE" => Some(CartType::Jane),
        "WF8" => Some(CartType::Wf8),
        "WD" => Some(CartType::Wd),
        "0FA0" => Some(CartType::ZeroFa0),
        "03E0" => Some(CartType::Zero3E0),
        "0840" => Some(CartType::Zero840),
        "EF" => Some(CartType::Ef),
        "DF" => Some(CartType::Df),
        "BF" => Some(CartType::Bf),
        "SB" => Some(CartType::Sb),
        "X07" => Some(CartType::X07),
        "MDM" => Some(CartType::Mdm),
        _ => None,
    }
}

struct VcsConsole {
    vcs: Vcs,
    title: String,
    rom_sha256: String,
    last_frame: IndexedFrame,
    tv: Television<VISIBLE_CLOCKS>,
}

/// The picture window shown from the full field the core emits: skip the
/// VBLANK lead-in after VSYNC, then show a fixed height so on-screen
/// geometry stays stable across kernels of varying line count. Values are
/// the standard NTSC/PAL picture regions (a TV crops to roughly this).
/// Frontend-only — the core keeps emitting every scanline.
struct DisplayWindow {
    skip: usize,
    height: usize,
}

fn display_window(standard: TvStandard) -> DisplayWindow {
    match standard {
        TvStandard::Ntsc => DisplayWindow {
            skip: 23,
            height: 228,
        },
        // SECAM shares PAL's 50 Hz, 312-line field geometry.
        TvStandard::Pal | TvStandard::Secam => DisplayWindow {
            skip: 32,
            height: 274,
        },
    }
}

fn indexed_frame(lines: &[[u8; VISIBLE_CLOCKS]], standard: TvStandard) -> IndexedFrame {
    let window = display_window(standard);
    let black = palette_index(0) as u8;
    let mut pixels = vec![black; window.height * VISIBLE_CLOCKS];
    for row in 0..window.height {
        if let Some(line) = lines.get(window.skip + row) {
            let dst = row * VISIBLE_CLOCKS;
            for (i, &p) in line.iter().enumerate() {
                pixels[dst + i] = palette_index(p) as u8;
            }
        }
    }
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height: window.height as u32,
        pixels: pixels.into(),
        palette: region_palette(standard),
    }
}

fn blank_frame() -> IndexedFrame {
    let height = display_window(TvStandard::Ntsc).height as u32;
    IndexedFrame::blank(
        VISIBLE_CLOCKS as u32,
        height,
        region_palette(TvStandard::Ntsc),
    )
}

/// A hex SHA-256 of the raw ROM image, taken at load (the cartridge does not
/// retain a plain board's image), so a save state can refuse a ROM it was not
/// written for.
fn rom_fingerprint(rom: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(rom);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The current displayed field as a save-state framebuffer blob — informational;
/// a restored console regenerates its display from the restored hardware.
fn state_frame(frame: &IndexedFrame) -> StateFrame {
    StateFrame {
        width: frame.width,
        height: Some(frame.height),
        format: PixelFormat::Indexed8,
        data: frame.pixels.to_vec(),
    }
}

/// Serialize the console's boundary state into a save file. `None` when the
/// console is mid-instruction — a save is only faithful at an instruction
/// boundary, where the CPU carries no micro-sequencer residue.
fn save_state_bytes(vcs: &Vcs, frame: &IndexedFrame, rom_sha256: &str) -> Option<Vec<u8>> {
    if !vcs.at_instruction_boundary() {
        return None;
    }
    let record = crate::snapshot::read_state(vcs);
    let memory = crate::snapshot::capture_memory(vcs);
    let saved = state_frame(frame);
    let meta = StateMeta {
        system: vcs_state_schema().system,
        rom_sha256: Some(rom_sha256),
        emulator: "missingno",
        emulator_version: env!("CARGO_PKG_VERSION"),
    };
    write_state_file(&meta, &record, &memory, Some(&saved)).ok()
}

/// Restore the console from a save file, rejecting a state for the wrong system
/// or ROM, an unsupported version, or a record that fails schema validation.
fn load_state_into(vcs: &mut Vcs, bytes: &[u8], rom_sha256: &str) -> Result<(), StateError> {
    use missingno_core::state_file::StateFileError;

    let schema = vcs_state_schema();
    let file = read_state_file(bytes).map_err(|error| match error {
        StateFileError::UnsupportedVersion(_) => StateError::VersionMismatch,
        _ => StateError::Corrupt,
    })?;
    if file.system != schema.system {
        return Err(StateError::WrongSystem);
    }
    if let Some(fingerprint) = &file.rom_sha256
        && fingerprint != rom_sha256
    {
        return Err(StateError::IncompatibleRom);
    }
    let record = schema
        .record_from(file.fields)
        .map_err(|_| StateError::Corrupt)?;
    crate::snapshot::restore(vcs, &record, &file.memory)
}

impl SystemConsole for VcsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let standard = self.vcs.tv_standard();
        // Drive the console scanline by scanline through the television, which
        // integrates VSYNC to decide the field. Bounded so a kernel that never
        // syncs cannot stall the emulation thread.
        let mut display = None;
        for _ in 0..FRAME_BUDGET_LINES {
            let line = self.vcs.step_scanline();
            if let Some(field) = self.tv.feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            }) {
                self.last_frame = indexed_frame(&field.lines, standard);
                display = Some(VideoFrame::Indexed(self.last_frame.clone()));
                break;
            }
        }
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.vcs.power_cycle();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.vcs, control, input);
    }

    fn console_switches(&self) -> &'static [ConsoleSwitch] {
        &CONSOLE_SWITCHES
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.vcs.drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(crate::board::AUDIO_COUPLING.high_pass())
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: self.vcs.tv_standard(),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        frame_interval(self.vcs.tv_standard())
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        Some(vcs_state_schema())
    }

    fn read_state(&self) -> Option<StateRecord> {
        Some(crate::snapshot::read_state(&self.vcs))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(&self.vcs, &self.last_frame, &self.rom_sha256)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        load_state_into(&mut self.vcs, bytes, &self.rom_sha256)
    }

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Ok(Box::new(VcsDebugger::new(
            crate::debugger::Debugger::new(self.vcs),
            self.title,
            self.rom_sha256,
            self.last_frame,
        )))
    }
}

/// Paddle 0's knob rides the first analog control id.
pub const PADDLE_CONTROL: ControlId = ControlId(8);

/// The family's reading of the shared control ids: the standard pad maps
/// onto the joystick and fire, Start/Select work the console switches,
/// and the paddle takes the axis.
fn apply_control(vcs: &mut Vcs, control: ControlId, input: ControlInput) {
    match input {
        ControlInput::Digital(pressed) => {
            let direction = match control.0 {
                0 => return vcs.set_console_reset(pressed),
                1 => return vcs.set_console_select(pressed),
                2 | 3 => return vcs.set_fire(pressed),
                4 => JoystickDirection::Up,
                5 => JoystickDirection::Down,
                6 => JoystickDirection::Left,
                7 => JoystickDirection::Right,
                // Latching console switches carry their level, not a press.
                9 => return vcs.set_difficulty(0, pressed),
                10 => return vcs.set_difficulty(1, pressed),
                11 => return vcs.set_color_mode(pressed),
                _ => return,
            };
            vcs.set_joystick(direction, pressed);
        }
        ControlInput::Axis(value) => {
            if control == PADDLE_CONTROL {
                vcs.set_paddle(0, value);
            }
        }
    }
}

/// The core's TIA palette for a standard as the screen path's shared RGB8 slice
/// — NTSC/PAL hue decode, or SECAM's luma-only 8 colours.
fn region_palette(standard: TvStandard) -> std::sync::Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTES: OnceLock<[std::sync::Arc<[RGB8]>; 3]> = OnceLock::new();
    let build = |standard| -> std::sync::Arc<[RGB8]> {
        crate::tia::palette(standard)
            .iter()
            .map(|&(r, g, b)| RGB8::new(r, g, b))
            .collect::<Vec<_>>()
            .into()
    };
    let cache = PALETTES.get_or_init(|| {
        [
            build(TvStandard::Ntsc),
            build(TvStandard::Pal),
            build(TvStandard::Secam),
        ]
    });
    let index = match standard {
        TvStandard::Ntsc => 0,
        TvStandard::Pal => 1,
        TvStandard::Secam => 2,
    };
    cache[index].clone()
}

#[derive(Clone, Default)]
pub struct VcsInspectState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    pub beam: u16,
    pub scanline: usize,
    pub timer: u8,
    pub timer_underflowed: bool,
    pub swcha: u8,
    pub swchb: u8,
    pub collisions: [u8; 8],
    /// TIA graphics registers, resolved to their object colours for the pixel
    /// strips (COLUPx is a hue the core owns).
    pub grp0: u8,
    pub grp0_reflect: bool,
    pub grp1: u8,
    pub grp1_reflect: bool,
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub pf_mirrored: bool,
    pub missile0: bool,
    pub missile1: bool,
    pub ball: bool,
    pub color_p0: RGB8,
    pub color_p1: RGB8,
    pub color_pf: RGB8,
    /// TIA audio: each channel's AUDC/AUDF/AUDV register bytes.
    pub audc: [u8; 2],
    pub audf: [u8; 2],
    pub audv: [u8; 2],
    /// The board and, on a DPC cart, its custom chip.
    pub cartridge: crate::cartridge::CartridgeInspect,
    pub frame: u64,
}

/// The VCS sidebar sections, shared by the live debugger (paused) and the
/// per-frame snapshot so the two agree by construction: the 6507 register file
/// and the TIA/RIOT state the inspection struct already carries.
pub fn vcs_sidebar_sections(state: &VcsInspectState) -> Vec<inspect::Section> {
    vec![
        cpu_section(state),
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

fn cpu_section(state: &VcsInspectState) -> inspect::Section {
    use inspect::{Register, RegisterGroup, SectionBlock, ValueStyle};

    let hex8 = |name, value: u8| Register {
        name,
        value: value as u32,
        bits: 8,
        style: ValueStyle::Hex,
        help: None,
    };
    let stack_pointer = 0x0100 | state.s as u16;
    let group = RegisterGroup {
        name: "cpu",
        registers: vec![
            hex8("a", state.a).help("accumulator"),
            hex8("x", state.x).help("X index register"),
            hex8("y", state.y).help("Y index register"),
            Register {
                name: "p",
                value: state.p as u32,
                bits: 8,
                style: ValueStyle::Flags(crate::debugger::MOS6502_FLAGS),
                help: Some("processor status flags"),
            },
        ],
    };
    inspect::Section {
        name: "CPU",
        summary: format!("pc {:04X} · sp {:04X}", state.pc, stack_pointer),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Pointers(vec![
                inspect::Pointer {
                    register: Register {
                        name: "pc",
                        value: state.pc as u32,
                        bits: 16,
                        style: ValueStyle::Hex,
                        help: Some("program counter"),
                    },
                    active: None,
                },
                inspect::Pointer {
                    register: Register {
                        name: "sp",
                        value: stack_pointer as u32,
                        bits: 16,
                        style: ValueStyle::Hex,
                        help: Some("stack pointer (offset into page 1)"),
                    },
                    active: None,
                },
            ]),
            SectionBlock::Rule,
            SectionBlock::Registers(group),
        ],
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

/// Bytes captured before the program counter, and the total span; the
/// remainder ahead covers the forward disassembly. The 6507 sees a 13-bit bus,
/// but the program counter and these reads wrap in the 16-bit space the peek
/// mirrors into.
const WINDOW_BEHIND: u16 = 128;
const WINDOW_LEN: u16 = 512;

/// The per-frame snapshot for the running view.
pub struct VcsSnapshot {
    pub state: VcsInspectState,
    memory: inspect::MemoryWindow,
    channel_waves: Option<Vec<missingno_core::waveform::ChannelWave>>,
}

impl VcsSnapshot {
    pub fn new(state: VcsInspectState) -> Self {
        VcsSnapshot {
            state,
            memory: inspect::MemoryWindow {
                base: 0,
                bytes: Vec::new(),
            },
            channel_waves: None,
        }
    }
}

impl InspectSnapshot for VcsSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        let s = &self.state;
        crate::debugger::cpu_register_groups(s.pc, s.a, s.x, s.y, s.s, s.p)
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        vcs_sidebar_sections(&self.state)
    }
    fn memory_window(&self) -> Option<&inspect::MemoryWindow> {
        Some(&self.memory)
    }
    fn pc(&self) -> Option<u32> {
        Some(self.state.pc as u32)
    }
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        Some(&missingno_6502::Mos6502)
    }
    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.channel_waves.clone()
    }
}

/// The VCS under its debugging backend, adapted to the seam. Symbols,
/// code/data logging, and watchpoints have no backend yet — the seam
/// defaults report them absent.
struct VcsDebugger {
    core: crate::debugger::Debugger,
    title: String,
    rom_sha256: String,
    last_frame: IndexedFrame,
    inspect: VcsInspectState,
    frame_count: u64,
}

impl VcsDebugger {
    fn new(
        core: crate::debugger::Debugger,
        title: String,
        rom_sha256: String,
        last_frame: IndexedFrame,
    ) -> Self {
        let mut this = VcsDebugger {
            core,
            title,
            rom_sha256,
            last_frame,
            inspect: VcsInspectState::default(),
            frame_count: 0,
        };
        this.refresh();
        this
    }

    /// Rebuild the inspection state from the console (peek-only).
    fn refresh(&mut self) {
        let vcs = self.core.console();
        let cpu = &vcs.cpu;
        let standard = vcs.tv_standard();
        let color = |byte: u8| {
            let (r, g, b) = crate::tia::palette(standard)[palette_index(byte)];
            RGB8::new(r, g, b)
        };
        let gfx = vcs.tia.graphics_registers();
        let audio = vcs.tia.audio_registers();
        self.inspect = VcsInspectState {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            s: cpu.s,
            p: cpu.p,
            pc: cpu.pc,
            beam: vcs.tia.beam(),
            scanline: vcs.scanline(),
            timer: vcs.peek(0x0284),
            timer_underflowed: vcs.peek(0x0285) & 0x80 != 0,
            swcha: vcs.peek(0x0280),
            swchb: vcs.peek(0x0282),
            collisions: std::array::from_fn(|i| vcs.peek(i as u16)),
            grp0: gfx.grp0,
            grp0_reflect: gfx.reflect_p0,
            grp1: gfx.grp1,
            grp1_reflect: gfx.reflect_p1,
            pf0: gfx.pf0,
            pf1: gfx.pf1,
            pf2: gfx.pf2,
            pf_mirrored: gfx.pf_mirrored,
            missile0: gfx.missile0,
            missile1: gfx.missile1,
            ball: gfx.ball,
            color_p0: color(gfx.color_p0),
            color_p1: color(gfx.color_p1),
            color_pf: color(gfx.color_pf),
            audc: [audio[0].control, audio[1].control],
            audf: [audio[0].frequency, audio[1].frequency],
            audv: [audio[0].volume, audio[1].volume],
            cartridge: vcs.cartridge().inspect(),
            frame: self.frame_count,
        };
    }

    fn display(&mut self, frame: Option<Frame>) -> Option<VideoFrame> {
        let frame = frame?;
        self.frame_count += 1;
        let standard = self.core.console().tv_standard();
        self.last_frame = indexed_frame(&frame.lines, standard);
        Some(VideoFrame::Indexed(self.last_frame.clone()))
    }
}

impl SystemDebugger for VcsDebugger {
    fn step(&mut self) -> StepOutcome {
        let frame = self.core.step();
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_over(&mut self) -> StepOutcome {
        let (frame, _) = self.core.step_over();
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_frame(&mut self) -> StepOutcome {
        use crate::debugger::Stop;
        let (frame, stop) = self.core.step_frame();
        let display = self.display(frame);
        self.refresh();
        match stop {
            Stop::Breakpoint => StepOutcome::Breakpoint { frame: display },
            Stop::Watch => match self.core.last_watch_hit() {
                Some(watch) => StepOutcome::WatchHit(watch),
                None => StepOutcome::Breakpoint { frame: display },
            },
            Stop::BudgetExhausted => StepOutcome::BudgetExhausted,
            Stop::Completed => StepOutcome::Completed { frame: display },
        }
    }

    fn tick_name(&self) -> Option<&'static str> {
        Some("colour clock")
    }

    fn step_tick(&mut self) {
        self.core.console_mut().step_clock();
        self.refresh();
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
    }

    fn reset(&mut self) {
        self.core.console_mut().power_cycle();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(self.core.console_mut(), control, input);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.core.console_mut().drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(crate::board::AUDIO_COUPLING.high_pass())
    }

    fn set_wave_capture(&mut self, on: bool) {
        self.core.console_mut().set_wave_capture(on);
    }

    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.core.console().channel_waves()
    }

    fn set_breakpoint(&mut self, address: u32) {
        self.core.set_breakpoint(address as u16);
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.core.clear_breakpoint(address as u16);
    }

    fn breakpoints(&self) -> BTreeSet<u32> {
        self.core.breakpoints().iter().map(|&a| a as u32).collect()
    }

    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        self.core.register_groups()
    }

    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        vcs_sidebar_sections(&self.inspect)
    }

    fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
        self.core.memory_regions()
    }

    fn peek(&self, address: u32) -> u8 {
        self.core.peek(address)
    }

    fn pc(&self) -> u32 {
        self.core.pc()
    }

    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        Some(self.core.instruction_set())
    }

    fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        self.core.present_address(address)
    }

    fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        self.core.locate_bank_window(bank, window)
    }

    fn watchables(&self) -> &'static [inspect::Watchable] {
        self.core.watchables()
    }

    fn add_watch(&mut self, watch: inspect::Watch) {
        self.core.add_watch(watch);
    }

    fn remove_watch(&mut self, watch: &inspect::Watch) {
        self.core.remove_watch(watch);
    }

    fn watches(&self) -> Vec<inspect::Watch> {
        self.core.watches()
    }

    fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.core.last_watch_hit()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn family_state(&self) -> &dyn std::any::Any {
        &self.inspect
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: self.core.console().tv_standard(),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        let mut state = self.inspect.clone();
        state.frame = frame;
        let base = state.pc.wrapping_sub(WINDOW_BEHIND);
        let bytes = (0..WINDOW_LEN)
            .map(|i| self.core.peek(base.wrapping_add(i) as u32))
            .collect();
        Box::new(VcsSnapshot {
            state,
            memory: inspect::MemoryWindow {
                base: base as u32,
                bytes,
            },
            channel_waves: self.core.console().channel_waves(),
        })
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.inspect.pc.into(),
            sp: (self.inspect.s as u16 | 0x0100).into(),
            video_label: "TIA",
            video_summary: format!(
                "beam {} · line {}",
                self.inspect.beam, self.inspect.scanline
            ),
            frame,
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        frame_interval(self.core.console().tv_standard())
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        Some(vcs_state_schema())
    }

    fn read_state(&self) -> Option<StateRecord> {
        Some(crate::snapshot::read_state(self.core.console()))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(self.core.console(), &self.last_frame, &self.rom_sha256)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let result = load_state_into(self.core.console_mut(), bytes, &self.rom_sha256);
        if result.is_ok() {
            self.refresh();
        }
        result
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(VcsConsole {
            vcs: self.core.into_console(),
            title: self.title,
            rom_sha256: self.rom_sha256,
            last_frame: self.last_frame,
            tv: Television::new(VSYNC_LOCK_LINES),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn video_out_reports_a_crt_with_the_carts_standard() {
        // A 4 KiB ROM whose reset vector points at its origin; the caller-
        // supplied standard maps straight onto the CRT descriptor.
        let mut rom = vec![0xEA; 0x1000];
        rom[0xFFC] = 0x00;
        rom[0xFFD] = 0xF0;
        for standard in [TvStandard::Ntsc, TvStandard::Pal, TvStandard::Secam] {
            let console =
                create_console(&rom, "test".into(), Some(standard), None).expect("console builds");
            match console.video_out() {
                DisplayTechnology::Crt {
                    standard: reported,
                    pixel_aspect,
                } => {
                    assert_eq!(reported, standard);
                    assert_eq!(pixel_aspect, PIXEL_ASPECT);
                }
                other => panic!("VCS should drive a CRT, got {other:?}"),
            }
        }
    }

    #[test]
    fn cartridge_section_shows_dpc_fetchers() {
        let cart =
            crate::cartridge::Cartridge::load(&vec![0u8; 0x2900], Some(CartType::Dpc), 3_579_545.0)
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
        assert_eq!(section.summary, "DPC (Pitfall II)");
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
        let cart =
            crate::cartridge::Cartridge::load(&vec![0u8; 0x2000], Some(CartType::F8), 3_579_545.0)
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
        let mut state = VcsInspectState::default();
        state.audc = [0x0C, 0x00];
        state.audf = [0x1F, 0x00];
        state.audv = [0x0A, 0x00];
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

    #[test]
    fn classify_fields_splits_ntsc_from_pal() {
        assert_eq!(classify_fields(&[42, 262, 262, 262]), TvStandard::Ntsc);
        assert_eq!(classify_fields(&[42, 312, 312, 312]), TvStandard::Pal);
        assert_eq!(classify_fields(&[]), TvStandard::Ntsc);
        // A long startup field doesn't sway the median.
        assert_eq!(
            classify_fields(&[45, 285, 282, 262, 262, 262]),
            TvStandard::Ntsc
        );
    }

    #[test]
    fn snapshot_register_groups_match_live() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        let rom = std::fs::read(&path).unwrap();
        let mut vcs = Vcs::new(&rom, TvStandard::Ntsc, None).unwrap();
        for _ in 0..64 {
            vcs.step_instruction();
        }
        let live = crate::debugger::Debugger::new(vcs);
        let cpu = &live.console().cpu;
        let state = VcsInspectState {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            s: cpu.s,
            p: cpu.p,
            pc: cpu.pc,
            ..Default::default()
        };
        let snapshot = VcsSnapshot::new(state);
        assert_eq!(
            format!("{:?}", live.register_groups()),
            format!("{:?}", snapshot.register_groups())
        );
    }

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        let rom = std::fs::read(&path).unwrap();
        let vcs = Vcs::new(&rom, TvStandard::Ntsc, None).unwrap();
        let mut debugger = VcsDebugger::new(
            crate::debugger::Debugger::new(vcs),
            "test".to_string(),
            String::new(),
            blank_frame(),
        );
        for _ in 0..64 {
            debugger.step();
        }
        let live = SystemDebugger::sidebar_sections(&debugger);
        let snapshot = debugger.snapshot(0);
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.sidebar_sections())
        );
    }

    #[test]
    fn probe_reads_ntsc_from_a_real_rom() {
        // A real 8 KB (F8) ROM, size-detected: build, run, and count its fields.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        assert_eq!(
            probe_tv_standard(&std::fs::read(&path).unwrap(), None),
            TvStandard::Ntsc
        );
    }
}
