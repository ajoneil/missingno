//! The SMS family's debugger seam: its inspection state and the machine
//! binding over the console. One owned state struct serves both the paused
//! view (refreshed after every step) and the per-frame snapshot the running
//! view renders from.

use std::sync::Arc;
use std::time::Duration;

use missingno_core::TvStandard;
use missingno_core::inspect::{
    BitColumn, BitRow, BitTable, RegisterGroup, Row, Section, SectionBlock, Sweep, SweepZone, Tone,
};
use missingno_core::machine::Machine;
use missingno_core::ports::{
    ControlDescriptor, PanelBehaviour, PanelControl, PeripheralDescriptor, PeripheralId, PlugError,
    PortDescriptor, PortId, Provider,
};
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, ControlSite, DebugView, InspectSnapshot, RunningStatus,
};
use missingno_core::video::{DisplayTechnology, Frame as DisplayFrame, IndexedFrame};
use rgb::RGB8;

use missingno_ti_psg::inspect::Registers as PsgRegisters;
use missingno_ti_psg::{NoiseMode, NoiseRate, Variant};
use missingno_zilog_z80::inspect::RegisterFile;

use crate::console::Sms;
use crate::vdp::{self, Frame};

/// What the board drives the PSG's CLOCK pin at.
const PSG_CLOCK_HZ: u32 = 3_579_545;

/// Instruction budget per frame step; generous over the ~15k typical.
const FRAME_BUDGET: u32 = 200_000;

/// NTSC pixel aspect at the VDP's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

const CODE_WINDOW_ROWS: usize = 10;

#[derive(Clone, Default)]
pub struct SmsInspectState {
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
    pub vdp_registers: [u8; 11],
    pub banks: [u8; 3],
    /// SN76489 PSG: the three tone periods, the four 4-bit attenuations, and
    /// the noise-control byte.
    pub psg_periods: [u16; 3],
    pub psg_volumes: [u8; 4],
    pub psg_noise: u8,
    /// Raw bytes at the program counter, hex-dumped until a Z80
    /// disassembler lands.
    pub code_window: Vec<(u16, [u8; 4])>,
    pub frame: u64,
}

/// The per-frame snapshot for the running view.
pub struct SmsSnapshot {
    pub state: SmsInspectState,
}

impl SmsSnapshot {
    pub fn new(state: SmsInspectState) -> Self {
        SmsSnapshot { state }
    }
}

impl InspectSnapshot for SmsSnapshot {
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
        sms_sidebar_sections(&self.state)
    }
}

/// The Z80 register file as one inspection group, shared by the live view and
/// the running snapshot. The part states its own layout.
fn cpu_register_groups(state: &SmsInspectState) -> Vec<RegisterGroup> {
    missingno_zilog_z80::inspect::register_groups(&RegisterFile {
        a: state.a,
        f: state.f,
        b: state.b,
        c: state.c,
        d: state.d,
        e: state.e,
        h: state.h,
        l: state.l,
        ix: state.ix,
        iy: state.iy,
        sp: state.sp,
        pc: state.pc,
    })
}

/// The SMS sidebar sections, shared by the live view and the running snapshot:
/// the Z80 register file, the Sega mapper's paged banks, and the VDP's
/// position, status, and registers.
fn sms_sidebar_sections(state: &SmsInspectState) -> Vec<Section> {
    let mut sections = missingno_core::inspect::default_sections(cpu_register_groups(state));
    sections.push(mapper_section(state));
    sections.push(vdp_section(state));
    sections.push(psg_section(state));
    sections
}

/// The Sega mapper's three bank registers ($FFFD–$FFFF), each selecting the ROM
/// bank paged into its 16 KiB slot.
fn mapper_section(state: &SmsInspectState) -> Section {
    Section {
        name: "Mapper",
        summary: format!(
            "{:02X} {:02X} {:02X}",
            state.banks[0], state.banks[1], state.banks[2]
        ),
        active: None,
        detail: None,
        blocks: vec![SectionBlock::Rows(vec![
            Row::value("slot0", format!("{:02X}", state.banks[0]))
                .help("bank register $FFFD — slot 0 ($0000–$3FFF)"),
            Row::value("slot1", format!("{:02X}", state.banks[1]))
                .help("bank register $FFFE — slot 1 ($4000–$7FFF)"),
            Row::value("slot2", format!("{:02X}", state.banks[2]))
                .help("bank register $FFFF — slot 2 ($8000–$BFFF)"),
        ])],
    }
}

/// The SN76489 PSG's register view, as the part states it. The board clocks it
/// from the same 3.579545 MHz the CPU runs on.
fn psg_section(state: &SmsInspectState) -> Section {
    missingno_ti_psg::inspect::section(
        &PsgRegisters {
            tone_periods: state.psg_periods,
            attenuations: state.psg_volumes,
            noise_mode: NoiseMode::from_control(state.psg_noise),
            noise_rate: NoiseRate::from_control(state.psg_noise),
            variant: Variant::SegaIntegrated,
        },
        PSG_CLOCK_HZ,
    )
}

fn vdp_section(state: &SmsInspectState) -> Section {
    use crate::vdp::{ACTIVE_LINES, DOTS_PER_LINE, LINES_PER_FRAME};

    let registers = state
        .vdp_registers
        .iter()
        .enumerate()
        .map(|(index, &value)| Row::value(format!("r{index}"), format!("{value:02X}")))
        .collect();
    // The active display occupies the first 192 lines; the border and vblank
    // share the rest. The dot cycle within a line carries no named zones.
    let line = Sweep::new("line", state.line as u32, LINES_PER_FRAME as u32)
        .zones(vec![
            SweepZone {
                name: "active",
                end: ACTIVE_LINES as u32,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "blank",
                end: LINES_PER_FRAME as u32,
                tone: Tone::Idle,
            },
        ])
        .help("VDP scanline — 0..191 active display, then border and vblank");
    let dot = Sweep::new("dot", state.dot as u32, DOTS_PER_LINE as u32)
        .help("VDP dot within the scanline");
    Section {
        name: "VDP",
        summary: format!("line {} · dot {}", state.line, state.dot),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Sweeps(vec![line, dot]),
            SectionBlock::Rule,
            SectionBlock::Table(status_table(state.vdp_status)),
            SectionBlock::Rule,
            SectionBlock::Rows(registers),
        ],
    }
}

/// The VDP status register's three documented flags; the low five bits carry
/// the fifth-sprite number.
fn status_table(status: u8) -> BitTable {
    BitTable {
        columns: vec![
            BitColumn::plain("int"),
            BitColumn::plain("ovr"),
            BitColumn::plain("col"),
        ],
        corner: None,
        rows: vec![BitRow {
            name: "status",
            bits: vec![status & 0x80 != 0, status & 0x40 != 0, status & 0x20 != 0],
            tone: Tone::Neutral,
        }],
    }
}

/// Master System media is recognised by its `.sms` file extension.
pub fn is_sms_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sms"))
}

pub struct SmsSystem;

impl Machine for SmsSystem {
    type Core = Sms;
    type Frame = Frame;
    type InspectState = SmsInspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    const RUN_BUDGET: u32 = 400_000;

    fn pc(sms: &Sms) -> u16 {
        sms.cpu.pc
    }

    fn peek(sms: &Sms, address: u16) -> u8 {
        sms.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn missingno_core::isa::InstructionSet> {
        Some(&missingno_zilog_z80::Z80)
    }

    fn step_instruction(sms: &mut Sms) {
        sms.step_instruction();
    }

    fn take_frame(sms: &mut Sms) -> Option<Frame> {
        sms.take_frame()
    }

    fn step_frame(sms: &mut Sms) -> Option<Frame> {
        sms.step_frame(FRAME_BUDGET)
    }

    fn power_cycle(sms: &mut Sms) {
        sms.power_cycle();
    }

    fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
        apply_control(sms, control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(_sms: &Sms, port: PortId) -> Option<PeripheralId> {
        (port == PAD_PORT).then_some(CONTROL_PAD)
    }

    fn plug(_sms: &mut Sms, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        match (port, peripheral) {
            (PAD_PORT, CONTROL_PAD) => Ok(()),
            (PAD_PORT, _) => Err(PlugError::NotAccepted),
            _ => Err(PlugError::UnknownPort),
        }
    }

    fn panel_controls() -> &'static [PanelControl] {
        PANEL
    }

    fn drain_audio_samples(sms: &mut Sms) -> Vec<(f32, f32)> {
        sms.drain_audio_samples()
    }

    fn video_out(_sms: &Sms) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: TvStandard::Ntsc,
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn display_frame(frame: &Frame) -> DisplayFrame {
        DisplayFrame::Indexed(IndexedFrame {
            width: vdp::PIXELS_PER_LINE as u32,
            height: vdp::ACTIVE_LINES as u32,
            pixels: frame.pixels.clone().into(),
            palette: cram_palette(&frame.cram),
        })
    }

    fn blank_display() -> DisplayFrame {
        DisplayFrame::Indexed(IndexedFrame::blank(
            vdp::PIXELS_PER_LINE as u32,
            vdp::ACTIVE_LINES as u32,
            cram_palette(&[0; 32]),
        ))
    }

    fn step_over_target(sms: &Sms) -> Option<u16> {
        // CALL and RST push a return path; run to the next address.
        let opcode = sms.peek(sms.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        is_call.then(|| sms.cpu.pc.wrapping_add(3))
    }

    fn inspect(sms: &Sms, frame_count: u64) -> SmsInspectState {
        let cpu = &sms.cpu;
        let mut code_window = Vec::with_capacity(CODE_WINDOW_ROWS);
        let mut address = cpu.pc;
        for _ in 0..CODE_WINDOW_ROWS {
            code_window.push((
                address,
                [
                    sms.peek(address),
                    sms.peek(address.wrapping_add(1)),
                    sms.peek(address.wrapping_add(2)),
                    sms.peek(address.wrapping_add(3)),
                ],
            ));
            address = address.wrapping_add(4);
        }
        // The mapper latches mirror into RAM, which inspection can read.
        let banks = [0, 1, 2].map(|slot| sms.peek(0xFFFD + slot as u16));
        SmsInspectState {
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
            line: sms.vdp.line(),
            dot: sms.vdp.dot(),
            vdp_status: sms.vdp.peek_status(),
            vdp_registers: sms.vdp.registers,
            banks,
            psg_periods: sms.psg.tone_periods(),
            psg_volumes: sms.psg.attenuations(),
            psg_noise: sms.psg.noise_control(),
            code_window,
            frame: frame_count,
        }
    }

    fn register_groups(state: &SmsInspectState) -> Vec<RegisterGroup> {
        cpu_register_groups(state)
    }

    fn sidebar_sections(state: &SmsInspectState) -> Vec<Section> {
        sms_sidebar_sections(state)
    }

    fn snapshot(state: &SmsInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(SmsSnapshot::new(state))
    }

    fn running_status(state: &SmsInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc.into(),
            sp: state.sp.into(),
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.line, state.dot),
            frame,
        }
    }
}

/// Resolve a CRAM snapshot (6-bit --BBGGRR) to display RGB.
fn cram_palette(cram: &[u8; 32]) -> Arc<[RGB8]> {
    cram.iter()
        .map(|&entry| {
            let channel = |bits: u8| (bits & 0x03) * 85;
            RGB8::new(channel(entry), channel(entry >> 2), channel(entry >> 4))
        })
        .collect()
}

/// The family's reading of the shared control ids: the pad maps onto the
/// port lines, and Start works the console Pause button (an NMI).
const PAD_PORT: PortId = PortId(0);
const CONTROL_PAD: PeripheralId = PeripheralId(0);

/// Pause is a button on the console itself, wired to the CPU's NMI rather than
/// to a controller line.
pub const PANEL: &[PanelControl] = &[PanelControl {
    role: ControlRole::Pause,
    label: "Pause",
    behaviour: PanelBehaviour::Momentary,
}];

const PAD_BUTTONS: &[ControlDescriptor] = &[
    ControlDescriptor::button(ControlRole::Action(0), "Button 1"),
    ControlDescriptor::button(ControlRole::Action(1), "Button 2"),
    ControlDescriptor::button(ControlRole::Up, "Up"),
    ControlDescriptor::button(ControlRole::Down, "Down"),
    ControlDescriptor::button(ControlRole::Left, "Left"),
    ControlDescriptor::button(ControlRole::Right, "Right"),
];

/// Only the first control pad is wired; the second port is not modelled.
pub const PORTS: &[PortDescriptor] = &[PortDescriptor {
    port: PAD_PORT,
    label: "Control pad",
    accepts: &[PeripheralDescriptor {
        id: CONTROL_PAD,
        label: "Control pad",
        provider: Provider::Console,
        controls: PAD_BUTTONS,
    }],
}];

fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    if control.site == ControlSite::Panel {
        if control.role == ControlRole::Pause && pressed {
            sms.cpu.trigger_nmi();
        }
        return;
    }
    if control.site != ControlSite::Port(PAD_PORT) {
        return;
    }
    let line = match control.role {
        ControlRole::Action(0) => 0x10,
        ControlRole::Action(1) => 0x20,
        ControlRole::Up => 0x01,
        ControlRole::Down => 0x02,
        ControlRole::Left => 0x04,
        ControlRole::Right => 0x08,
        _ => return,
    };
    if pressed {
        sms.port_dc &= !line;
    } else {
        sms.port_dc |= line;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psg_section_reports_registers_and_audibility() {
        let mut state = SmsInspectState::default();
        state.psg_periods = [0x1FE, 0, 0];
        // Tone 1 audible (attenuation 0), the rest muted at $F.
        state.psg_volumes = [0x00, 0x0F, 0x0F, 0x0F];
        state.psg_noise = 0x05;
        let section = psg_section(&state);
        assert_eq!(section.name, "PSG");

        let rows: Vec<&Row> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            rows.iter().find(|r| r.label == "per1").map(|r| &r.value),
            Some(&"1FE (219 Hz)".to_string())
        );
        // Tone 1's pip is lit (attenuation below $F); tone 2's is dim.
        assert_eq!(
            rows.iter()
                .find(|r| r.label == "tone 1")
                .and_then(|r| r.active),
            Some(true)
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.label == "tone 2")
                .and_then(|r| r.active),
            Some(false)
        );
    }

    #[test]
    fn mapper_section_reports_each_slot_bank() {
        let mut state = SmsInspectState::default();
        state.banks = [0x01, 0x02, 0x03];
        let section = mapper_section(&state);
        assert_eq!(section.name, "Mapper");

        let rows: Vec<&Row> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        for (label, value) in [("slot0", "01"), ("slot1", "02"), ("slot2", "03")] {
            assert_eq!(
                rows.iter().find(|r| r.label == label).map(|r| &r.value),
                Some(&value.to_string())
            );
        }
    }
}
