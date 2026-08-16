//! The SG-1000's debugger seam: its inspection state and the stepping-system
//! binding over the console. One owned state struct serves both the paused
//! view (refreshed after every step) and the per-frame snapshot the running
//! view renders from.

pub mod graphics;

use std::sync::Arc;
use std::time::Duration;

use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{
    BitColumn, BitRow, BitTable, ColorSwatch, Concept, Detail, Register, RegisterGroup,
    RegisterPurpose, Row, Section, SectionBlock, SwatchRow, Sweep, SweepZone, Tone, ValueStyle,
};
use missingno_core::ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, PeripheralDescriptor,
    PeripheralId, PlugError, PortDescriptor, PortId, Provider,
};
use missingno_core::stepping::SteppingSystem;
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, DebugView, InspectSnapshot, RunningStatus,
};
use missingno_core::video::IndexedFrame;
use missingno_core::waveform::ChannelWave;
use missingno_ti_vdp::{ACTIVE_LINES, Frame, Mode, Standard, VISIBLE_WIDTH, Vdp};
use rgb::RGB8;

use crate::console::{JOY1, JOY2, Sg1000};

/// T-state budget per frame step; a frame is 59,736 of them, and only a wait
/// chain can stretch one.
const FRAME_BUDGET: u32 = 4 * TSTATES_PER_FRAME;

/// One NTSC frame: 262 lines of 228 T-states at 3.579545 MHz.
const TSTATES_PER_FRAME: u32 = 228 * 262;

/// NTSC pixel aspect at the VDP's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

/// Dots per counter line; the chip states the windows inside the raster.
const DOTS_PER_LINE: u32 = 342;

const CODE_WINDOW_ROWS: usize = 10;

/// The clock the Z80 and the PSG's CLOCK pin share.
const CLOCK_HZ: f32 = 3_579_545.0;
/// The tone counter divides the input clock by 32 per period count.
const TONE_DIVISOR: f32 = 32.0;
/// What a zero period register counts, on the discrete part this board fits.
const ZERO_PERIOD_COUNT: u16 = 0x400;
/// The attenuator ladder's nominal step.
const DECIBELS_PER_STEP: u8 = 2;
/// The attenuation that switches a channel off.
const MUTE_ATTENUATION: u8 = 0x0F;
/// The PSG's fourth channel.
const NOISE_CHANNEL: usize = 3;

/// R1's magnification bit; SIZE has an accessor of its own on the chip.
const R1_MAG: u8 = 0x01;
/// R7's backdrop colour sits in its low nibble.
const BACKDROP_MASK: u8 = 0x0F;
/// The status register's low five bits carry the sprite-scan counter.
const SCAN_COUNTER_MASK: u8 = 0x1F;

/// The canonical TI datasheet palette. The chip stops at colour indices, so
/// resolving them to RGB is presentation policy the console states; index 0 is
/// the all-planes-transparent external-video pass-through and presents black.
const TI_PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 0],
    [33, 200, 66],
    [94, 220, 120],
    [84, 85, 237],
    [125, 118, 252],
    [212, 82, 77],
    [66, 235, 245],
    [252, 85, 84],
    [255, 121, 120],
    [212, 193, 84],
    [230, 206, 128],
    [33, 176, 59],
    [201, 91, 186],
    [204, 204, 204],
    [255, 255, 255],
];

/// What the chip's own accessors make of the register file: the mode R0/R1
/// select, the five table bases R2-R6 point at, and R1's sprite geometry.
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
}

impl VdpLayout {
    fn of(vdp: &Vdp) -> VdpLayout {
        VdpLayout {
            mode: vdp.mode(),
            name_table: vdp.name_table_base(),
            pattern_table: vdp.pattern_table_base(),
            colour_table: vdp.colour_table_base(),
            sprite_attributes: vdp.sprite_attribute_base(),
            sprite_patterns: vdp.sprite_pattern_base(),
            sprites_16x16: vdp.sprites_16x16(),
            magnified: vdp.registers()[1] & R1_MAG != 0,
        }
    }
}

impl Default for VdpLayout {
    /// What a power-on chip reads: its register file is zeroed.
    fn default() -> VdpLayout {
        VdpLayout::of(&Vdp::new(Standard::Ntsc))
    }
}

#[derive(Clone, Default)]
pub struct Sg1000InspectState {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub line: u16,
    pub dot: u16,
    pub vdp_status: u8,
    pub vdp_registers: [u8; 8],
    pub vdp_layout: VdpLayout,
    /// SN76489AN: the three tone periods, the four 4-bit attenuations, and the
    /// noise-control byte.
    pub psg_periods: [u16; 3],
    pub psg_volumes: [u8; 4],
    pub psg_noise: u8,
    /// Raw bytes at the program counter, for the code window.
    pub code_window: Vec<(u16, [u8; 4])>,
    pub frame: u64,
}

/// The per-frame snapshot for the running view.
pub struct Sg1000Snapshot {
    pub state: Sg1000InspectState,
}

impl Sg1000Snapshot {
    pub fn new(state: Sg1000InspectState) -> Self {
        Sg1000Snapshot { state }
    }
}

impl InspectSnapshot for Sg1000Snapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        cpu_register_groups(&self.state)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        sg1000_sidebar_sections(&self.state)
    }
}

/// The Z80 register file as one inspection group, shared by the live view and
/// the running snapshot.
fn cpu_register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
    use RegisterPurpose::{PairHigh, PairLow, ProgramCounter, StackPointer};

    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", state.a as u32, 8)
                .help("accumulator")
                .purpose(PairHigh("af")),
            Register {
                name: "f",
                value: state.f as u32,
                bits: 8,
                style: ValueStyle::Flags(missingno_zilog_z80::flags::NAMES),
                help: Some("flags register"),
                purpose: Some(PairLow("af")),
                active: None,
            },
            hex("b", state.b as u32, 8)
                .help("general register B (high byte of BC)")
                .purpose(PairHigh("bc")),
            hex("c", state.c as u32, 8)
                .help("general register C (low byte of BC)")
                .purpose(PairLow("bc")),
            hex("d", state.d as u32, 8)
                .help("general register D (high byte of DE)")
                .purpose(PairHigh("de")),
            hex("e", state.e as u32, 8)
                .help("general register E (low byte of DE)")
                .purpose(PairLow("de")),
            hex("h", state.h as u32, 8)
                .help("general register H (high byte of HL)")
                .purpose(PairHigh("hl")),
            hex("l", state.l as u32, 8)
                .help("general register L (low byte of HL)")
                .purpose(PairLow("hl")),
            hex("ix", state.ix as u32, 16).help("index register IX"),
            hex("iy", state.iy as u32, 16).help("index register IY"),
            hex("sp", state.sp as u32, 16)
                .help("stack pointer")
                .purpose(StackPointer),
            hex("pc", state.pc as u32, 16)
                .help("program counter")
                .purpose(ProgramCounter),
        ],
    }]
}

/// The sidebar sections, shared by the live view and the running snapshot: the
/// Z80 register file, the VDP's position/status/registers, and the PSG's
/// channels. The board has no mapper to show.
fn sg1000_sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
    vec![
        missingno_core::inspect::cpu_section(cpu_register_groups(state)),
        vdp_section(state),
        psg_section(state),
    ]
}

/// The SN76489AN: three tone channels and one noise channel, each with an
/// audibility pip — attenuation $F fully mutes, so anything below it is
/// audible. The period and attenuation rows carry the arithmetic a reader
/// would otherwise do by hand.
fn psg_section(state: &Sg1000InspectState) -> Section {
    let audible = state
        .psg_volumes
        .iter()
        .filter(|&&attenuation| attenuation != MUTE_ATTENUATION)
        .count();
    Section {
        name: "PSG",
        summary: format!("{audible}/4 audible"),
        active: None,
        detail: None,
        blocks: vec![
            tone_rows(state, 0),
            SectionBlock::Rule,
            tone_rows(state, 1),
            SectionBlock::Rule,
            tone_rows(state, 2),
            SectionBlock::Rule,
            noise_rows(state),
        ],
    }
}

/// One tone channel: the period register with the tone it produces, and the
/// attenuation with its place on the ladder.
fn tone_rows(state: &Sg1000InspectState, channel: usize) -> SectionBlock {
    let period = state.psg_periods[channel];
    let attenuation = state.psg_volumes[channel];
    SectionBlock::Rows(vec![
        Row::flag(
            format!("tone {}", channel + 1),
            attenuation != MUTE_ATTENUATION,
        )
        .help("channel audible — attenuation below $F"),
        Row::value(format!("per{}", channel + 1), tone_label(period))
            .help("10-bit period register; the tone is the 3.579545 MHz clock ÷ 32n"),
        Row::value(
            format!("att{}", channel + 1),
            attenuation_label(attenuation),
        )
        .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// The noise channel: what its feedback network and its counter's rate are
/// set to, and the same attenuator as a tone channel's.
fn noise_rows(state: &Sg1000InspectState) -> SectionBlock {
    let attenuation = state.psg_volumes[NOISE_CHANNEL];
    SectionBlock::Rows(vec![
        Row::flag("noise", attenuation != MUTE_ATTENUATION)
            .help("channel audible — attenuation below $F"),
        Row::value("mode", noise_mode(state.psg_noise))
            .help("feedback network (noise control bit 2)"),
        Row::value("rate", noise_rate(state.psg_noise))
            .help("shift rate (noise control bits 0-1) — a fixed division, or tone 3's period"),
        Row::value("att4", attenuation_label(attenuation))
            .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// A period register beside the tone it produces.
fn tone_label(period: u16) -> String {
    format!(
        "{period:03X} ({} Hz)",
        tone_frequency(period).round() as u32
    )
}

/// The ÷16 prescaler feeds a counter that reloads `n` and toggles its
/// flip-flop each borrow, so the tone is the input clock over 32n.
fn tone_frequency(period: u16) -> f32 {
    let count = if period == 0 {
        ZERO_PERIOD_COUNT
    } else {
        period
    };
    CLOCK_HZ / (TONE_DIVISOR * f32::from(count))
}

/// An attenuation register beside the attenuation it sets.
fn attenuation_label(attenuation: u8) -> String {
    if attenuation >= MUTE_ATTENUATION {
        format!("{attenuation:X} (off)")
    } else if attenuation == 0 {
        format!("{attenuation:X} (0 dB)")
    } else {
        format!("{attenuation:X} (-{} dB)", attenuation * DECIBELS_PER_STEP)
    }
}

fn noise_mode(control: u8) -> &'static str {
    if control & 0x04 != 0 {
        "white"
    } else {
        "periodic"
    }
}

fn noise_rate(control: u8) -> &'static str {
    match control & 0x03 {
        0 => "clock ÷ 512",
        1 => "clock ÷ 1024",
        2 => "clock ÷ 2048",
        _ => "tone 3",
    }
}

/// The TMS9918A at register level: where the raster stands, what the status
/// latches hold, the five tables R2-R6 point at, and the backdrop R7 paints.
fn vdp_section(state: &Sg1000InspectState) -> Section {
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
            SectionBlock::Sweeps(vec![line_sweep(state.line), dot]),
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
            backdrop_swatches(state.vdp_registers[7]),
            SectionBlock::Rule,
            SectionBlock::Rows(registers),
        ],
    }
}

/// The line counter across the frame. The visible raster is not contiguous in
/// counter order — the top border rides the wrap — so the border shows as two
/// zones with the blanking lines between them.
fn line_sweep(line: u16) -> Sweep {
    let lines_per_frame = Standard::Ntsc.lines_per_frame() as u32;
    let display = ACTIVE_LINES as u32;
    let bottom = display + Standard::Ntsc.bottom_border() as u32;
    let top = lines_per_frame - Standard::Ntsc.top_border() as u32;
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
fn backdrop_swatches(r7: u8) -> SectionBlock {
    let index = r7 & BACKDROP_MASK;
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

/// SG-1000 media is recognised by its `.sg` file extension.
pub fn is_sg1000_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sg"))
}

pub struct Sg1000System;

impl SteppingSystem for Sg1000System {
    type Core = Sg1000;
    type Frame = Frame;
    type InspectState = Sg1000InspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    const RUN_BUDGET: u32 = 400_000;
    const PIXEL_ASPECT: f32 = PIXEL_ASPECT;

    fn pc(sg: &Sg1000) -> u16 {
        sg.cpu.pc
    }

    fn peek(sg: &Sg1000, address: u16) -> u8 {
        sg.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn missingno_core::isa::InstructionSet> {
        Some(&missingno_zilog_z80::Z80)
    }

    fn step_instruction(sg: &mut Sg1000) {
        sg.step_instruction();
    }

    fn take_frame(sg: &mut Sg1000) -> Option<Frame> {
        sg.take_frame()
    }

    fn step_frame(sg: &mut Sg1000) -> Option<Frame> {
        sg.step_frame(FRAME_BUDGET)
    }

    fn power_cycle(sg: &mut Sg1000) {
        sg.power_cycle();
    }

    fn apply_control(sg: &mut Sg1000, control: ControlId, input: ControlInput) {
        sg.apply_control(control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(_sg: &Sg1000, port: PortId) -> Option<PeripheralId> {
        matches!(port, JOY1 | JOY2).then_some(CONTROL_PAD)
    }

    fn plug(_sg: &mut Sg1000, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        match (port, peripheral) {
            (JOY1 | JOY2, CONTROL_PAD) => Ok(()),
            (JOY1 | JOY2, _) => Err(PlugError::NotAccepted),
            _ => Err(PlugError::UnknownPort),
        }
    }

    fn panel_controls() -> &'static [PanelControl] {
        PANEL
    }

    fn drain_audio_samples(sg: &mut Sg1000) -> Vec<(f32, f32)> {
        sg.drain_audio_samples()
    }

    fn indexed_frame(frame: &Frame) -> IndexedFrame {
        IndexedFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            pixels: frame.pixels.as_slice().into(),
            palette: ti_palette(),
        }
    }

    fn blank_frame() -> IndexedFrame {
        IndexedFrame::blank(
            VISIBLE_WIDTH as u32,
            Standard::Ntsc.visible_lines() as u32,
            ti_palette(),
        )
    }

    fn step_over_target(sg: &Sg1000) -> Option<u16> {
        // CALL and RST push a return path; run to the next address.
        let opcode = sg.peek(sg.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        is_call.then(|| sg.cpu.pc.wrapping_add(3))
    }

    fn inspect(sg: &Sg1000, frame_count: u64) -> Sg1000InspectState {
        let cpu = &sg.cpu;
        let mut code_window = Vec::with_capacity(CODE_WINDOW_ROWS);
        let mut address = cpu.pc;
        for _ in 0..CODE_WINDOW_ROWS {
            code_window.push((
                address,
                [
                    sg.peek(address),
                    sg.peek(address.wrapping_add(1)),
                    sg.peek(address.wrapping_add(2)),
                    sg.peek(address.wrapping_add(3)),
                ],
            ));
            address = address.wrapping_add(4);
        }
        Sg1000InspectState {
            a: cpu.a,
            f: cpu.f,
            b: cpu.b,
            c: cpu.c,
            d: cpu.d,
            e: cpu.e,
            h: cpu.h,
            l: cpu.l,
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
            line: sg.vdp().line(),
            dot: sg.vdp().dot(),
            vdp_status: sg.vdp().peek_status(),
            vdp_registers: *sg.vdp().registers(),
            vdp_layout: VdpLayout::of(sg.vdp()),
            psg_periods: sg.psg().tone_periods(),
            psg_volumes: sg.psg().attenuations(),
            psg_noise: sg.psg().noise_control(),
            code_window,
            frame: frame_count,
        }
    }

    fn set_wave_capture(sg: &mut Sg1000, on: bool) {
        sg.set_wave_capture(on);
    }

    fn channel_waves(sg: &Sg1000) -> Option<Vec<ChannelWave>> {
        sg.channel_waves()
    }

    fn set_graphics_capture(sg: &mut Sg1000, on: bool) {
        sg.set_graphics_capture(on);
    }

    fn graphics_view(sg: &Sg1000) -> Option<GraphicsView> {
        graphics::graphics_view(sg)
    }

    fn register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
        cpu_register_groups(state)
    }

    fn sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
        sg1000_sidebar_sections(state)
    }

    fn snapshot(state: &Sg1000InspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(Sg1000Snapshot::new(state))
    }

    fn running_status(state: &Sg1000InspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc.into(),
            sp: state.sp.into(),
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.line, state.dot),
            frame,
        }
    }
}

fn ti_palette() -> Arc<[RGB8]> {
    (0..TI_PALETTE.len() as u8).map(ti_colour).collect()
}

fn ti_colour(index: u8) -> RGB8 {
    let [r, g, b] = TI_PALETTE[index as usize & 0x0F];
    RGB8::new(r, g, b)
}

const CONTROL_PAD: PeripheralId = PeripheralId(0);

/// Pause is a switch on the console itself, wired to /NMI rather than to a
/// controller line.
pub const PANEL: &[PanelControl] = &[PanelControl {
    role: ControlRole::Pause,
    label: "Pause",
    behaviour: PanelBehaviour::Momentary,
}];

const PAD_BUTTONS: &[ControlDescriptor] = &[
    button(ControlRole::Action(0), "Button 1"),
    button(ControlRole::Action(1), "Button 2"),
    button(ControlRole::Up, "Up"),
    button(ControlRole::Down, "Down"),
    button(ControlRole::Left, "Left"),
    button(ControlRole::Right, "Right"),
];

const fn button(role: ControlRole, label: &'static str) -> ControlDescriptor {
    ControlDescriptor {
        role,
        label,
        kind: ControlKind::Button,
    }
}

const fn pad_port(port: PortId, label: &'static str) -> PortDescriptor {
    PortDescriptor {
        port,
        label,
        accepts: &[PeripheralDescriptor {
            id: CONTROL_PAD,
            label: "Control pad",
            provider: Provider::Console,
            controls: PAD_BUTTONS,
        }],
    }
}

/// Both joystick sites the multiplexer pair presents. Player 1's stick is
/// wired to the board rather than to a connector, but it reads through the
/// same mux as player 2's.
pub const PORTS: &[PortDescriptor] = &[
    pad_port(JOY1, "Control pad 1"),
    pad_port(JOY2, "Control pad 2"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every label/value row a section carries, blocks flattened.
    fn rows(section: &Section) -> Vec<&Row> {
        section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn value_of<'a>(rows: &[&'a Row], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|row| row.label == label)
            .map(|row| row.value.as_str())
    }

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
    fn tone_frequency_is_the_clock_over_thirty_two_counts() {
        // 3.579545 MHz / (32 · 254) = 440.4 Hz.
        assert_eq!(tone_frequency(0x0FE).round() as u32, 440);
        assert_eq!(tone_frequency(0x1FE).round() as u32, 219);
        // A zero register counts as $400: 3.579545 MHz / (32 · 1024).
        assert_eq!(tone_frequency(0).round() as u32, 109);
    }

    #[test]
    fn attenuation_reads_as_decibels() {
        assert_eq!(attenuation_label(0x0), "0 (0 dB)");
        assert_eq!(attenuation_label(0x1), "1 (-2 dB)");
        assert_eq!(attenuation_label(0x5), "5 (-10 dB)");
        assert_eq!(attenuation_label(0xF), "F (off)");
    }

    #[test]
    fn psg_section_pairs_registers_with_their_arithmetic() {
        let state = Sg1000InspectState {
            psg_periods: [0x0FE, 0x1FE, 0],
            // Tone 1 audible (attenuation 0), the rest muted at $F.
            psg_volumes: [0x00, 0x0F, 0x0F, 0x0F],
            psg_noise: 0x05,
            ..Sg1000InspectState::default()
        };
        let section = psg_section(&state);
        assert_eq!(section.name, "PSG");
        assert_eq!(section.summary, "1/4 audible");

        let rows = rows(&section);
        assert_eq!(value_of(&rows, "per1"), Some("0FE (440 Hz)"));
        assert_eq!(value_of(&rows, "att1"), Some("0 (0 dB)"));
        assert_eq!(value_of(&rows, "per2"), Some("1FE (219 Hz)"));
        assert_eq!(value_of(&rows, "att2"), Some("F (off)"));
        // Noise control $05: white feedback, the input clock over 1024.
        assert_eq!(value_of(&rows, "mode"), Some("white"));
        assert_eq!(value_of(&rows, "rate"), Some("clock ÷ 1024"));
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 1")
                .and_then(|row| row.active),
            Some(true)
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 2")
                .and_then(|row| row.active),
            Some(false)
        );
    }

    #[test]
    fn vdp_section_reports_all_eight_registers() {
        let state = Sg1000InspectState {
            vdp_registers: [0x00, 0x60, 0x0E, 0xFF, 0x03, 0x76, 0x03, 0x01],
            ..Sg1000InspectState::default()
        };
        let section = vdp_section(&state);
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
            ..Sg1000InspectState::default()
        };
        let section = vdp_section(&state);
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

    #[test]
    fn the_sidebar_names_the_cpu_and_both_chips() {
        let sections = sg1000_sidebar_sections(&Sg1000InspectState::default());
        let names: Vec<&str> = sections.iter().map(|section| section.name).collect();
        assert_eq!(names, ["CPU", "VDP", "PSG"]);
    }

    #[test]
    fn the_cpu_section_sets_the_pointers_and_pairs_apart_from_the_file() {
        let state = Sg1000InspectState {
            pc: 0x1234,
            sp: 0xDFF0,
            a: 0x5A,
            f: 0x0F,
            b: 0xC0,
            c: 0xDE,
            ..Sg1000InspectState::default()
        };
        let section = missingno_core::inspect::cpu_section(cpu_register_groups(&state));
        assert_eq!(section.summary, "pc 1234 · sp DFF0");

        let pointers: Vec<&str> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Pointers(pointers) => Some(pointers),
                _ => None,
            })
            .flatten()
            .map(|pointer| pointer.register.name)
            .collect();
        assert_eq!(pointers, ["pc", "sp"]);

        let pairs: Vec<u32> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Pairs(pairs) => Some(pairs),
                _ => None,
            })
            .flatten()
            .map(|pair| pair.combined())
            .collect();
        assert_eq!(pairs, [0x5A0F, 0xC0DE, 0x0000, 0x0000]);

        let file: Vec<&str> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Registers(group) => Some(&group.registers),
                _ => None,
            })
            .flatten()
            .map(|register| register.name)
            .collect();
        assert_eq!(file, ["ix", "iy"]);
    }

    #[test]
    fn media_is_recognised_by_extension() {
        assert!(is_sg1000_rom(std::path::Path::new("game.sg")));
        assert!(is_sg1000_rom(std::path::Path::new("GAME.SG")));
        assert!(!is_sg1000_rom(std::path::Path::new("game.sms")));
    }
}
