//! The SG-1000 save-state bridge: it maps the board's three chips onto the
//! hardware-named [`SystemStateSchema`](missingno_core::state::SystemStateSchema)
//! and back. Capture reads the console into a [`StateRecord`] keyed by the
//! schema's field names; restore parses a record and rebuilds the console in
//! place at an instruction boundary.
//!
//! A capture is refused anywhere but an instruction boundary, where the Z80's
//! sequencer is absent. The VDP and the PSG run on a finer grid than the CPU
//! and are captured wherever that boundary lands them, counters and in-flight
//! access included, so a restore continues the field the save was taken in.
//! The rows that field has already emitted travel as the state's framebuffer —
//! nothing later can reconstruct them.

use missingno_core::machine::BoundaryState;
use missingno_core::state::{PixelFormat, StateRecord, StateValue};
use missingno_core::state_file::StateFrame;
use missingno_core::system::StateError;
use missingno_ti_psg::{
    Channel, NoiseMode, NoiseRate, NoiseState, PsgState, RegisterKind, ToneState,
};
use missingno_ti_vdp::{
    AccessState, PortState, PortTransfer, ScanStop, ScannerState, SegmentState, StatusState,
    VdpState,
};
use missingno_zilog_z80::{CpuState, InterruptMode};

use crate::console::{BoardState, Sg1000};

/// Read the whole board into a schema-keyed record; `None` mid-instruction,
/// where the CPU carries residue no field names.
pub fn read_state(sg: &Sg1000) -> Option<StateRecord> {
    let cpu = sg.cpu.boundary_state()?;
    let vdp = sg.vdp().boundary_state();
    let psg = sg.psg().boundary_state();
    let board = sg.board_state();

    let mut record = StateRecord::new();
    write_cpu(&mut record, &cpu);
    write_vdp(&mut record, &vdp);
    write_psg(&mut record, &psg);
    write_board(&mut record, &board);
    Some(record)
}

/// The whole boundary state a save file carries.
pub fn capture(sg: &Sg1000) -> Result<BoundaryState, StateError> {
    let record = read_state(sg).ok_or(StateError::NotAtBoundary)?;
    Ok(BoundaryState {
        record,
        memory: capture_memory(sg),
        frame: Some(raster(sg)),
    })
}

/// The byte regions beside the record: the work RAM, the VDP's DRAM, and the
/// two line buffers under the raster.
pub fn capture_memory(sg: &Sg1000) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("work_ram", sg.work_ram().to_vec()),
        ("vram", sg.vdp().vram().to_vec()),
        ("vdp_line", sg.vdp().line_buffer().to_vec()),
        ("vdp_sprite_plane", sg.vdp().sprite_plane().to_vec()),
    ]
}

/// The field being emitted, as the state file's framebuffer carries it.
pub fn raster(sg: &Sg1000) -> StateFrame {
    let frame = sg.vdp().frame();
    StateFrame {
        width: frame.width as u32,
        height: Some(frame.height as u32),
        format: PixelFormat::Indexed8,
        data: frame.pixels.clone(),
    }
}

// ── Capture ───────────────────────────────────────────────────────

fn write_cpu(r: &mut StateRecord, cpu: &CpuState) {
    r.set("a", cpu.a)
        .set("f", cpu.f)
        .set("b", cpu.b)
        .set("c", cpu.c)
        .set("d", cpu.d)
        .set("e", cpu.e)
        .set("h", cpu.h)
        .set("l", cpu.l)
        .set("a_alt", cpu.a_alt)
        .set("f_alt", cpu.f_alt)
        .set("b_alt", cpu.b_alt)
        .set("c_alt", cpu.c_alt)
        .set("d_alt", cpu.d_alt)
        .set("e_alt", cpu.e_alt)
        .set("h_alt", cpu.h_alt)
        .set("l_alt", cpu.l_alt)
        .set("ix", cpu.ix)
        .set("iy", cpu.iy)
        .set("sp", cpu.sp)
        .set("pc", cpu.pc)
        .set("i", cpu.i)
        .set("r", cpu.r)
        .set("iff1", cpu.iff1)
        .set("iff2", cpu.iff2)
        .set("im", interrupt_mode_number(cpu.interrupt_mode))
        .set("halted", cpu.halted)
        .set("wz", cpu.wz)
        .set("q", cpu.q)
        .set("p", cpu.p)
        .set("ei_pending", cpu.ei_pending)
        .set("flags_touched", cpu.flags_touched)
        .set("nmi_pending", cpu.nmi_pending)
        .set("irq_line", cpu.irq_line)
        .set("irq_sampled", cpu.irq_sampled)
        .set("address_bus", cpu.address_bus);
}

fn write_vdp(r: &mut StateRecord, vdp: &VdpState) {
    for (index, name) in REGISTER_FIELDS.into_iter().enumerate() {
        r.set(name, vdp.registers[index]);
    }
    let (transfer_write, transfer_data) = transfer_parts(vdp.port.transfer);
    let (prior_write, prior_data) = transfer_parts(vdp.port.prior_transfer);
    let (stop_kind, stop_index) = stop_parts(vdp.scanner.stop);

    r.set("vdp_address", vdp.port.address)
        .set("vdp_frame_flag", vdp.status.frame)
        .set("vdp_fifth_sprite_flag", vdp.status.fifth_sprite)
        .set("vdp_coincidence_flag", vdp.status.coincidence)
        .set("vdp_fifth_sprite_index", vdp.status.sprite_field)
        .set("vdp_line", vdp.line)
        .set("vdp_line_xtal", vdp.line_xtal as u16)
        .set("vdp_fields_completed", vdp.fields_completed as u32)
        .set("vdp_awaiting_second_byte", vdp.port.awaiting_second_byte)
        .set("vdp_read_buffer", vdp.port.read_buffer)
        .set("vdp_transfer_write", transfer_write)
        .set("vdp_transfer_data", transfer_data)
        .set("vdp_prior_transfer_write", prior_write)
        .set("vdp_prior_transfer_data", prior_data)
        .set("vdp_transfer_written_ago", vdp.port.transfer_written_ago)
        .set("vdp_pending_address", vdp.port.pending_address)
        .set("vdp_pending_flag", vdp.port.pending_flag)
        .set("vdp_access_active", vdp.port.access.is_some())
        .set(
            "vdp_access_address",
            vdp.port.access.map_or(0, |access| access.address),
        )
        .set(
            "vdp_access_claimed_ago",
            vdp.port.access.map_or(0, |access| access.claimed_ago),
        )
        .set("vdp_frame_flag_set_ago", vdp.status.frame_set_ago)
        .set("vdp_fifth_sprite_set_ago", vdp.status.fifth_sprite_set_ago)
        .set("vdp_scan_counter", vdp.scanner.counter)
        .set("vdp_scan_stop_kind", stop_kind)
        .set("vdp_scan_stop_index", stop_index)
        .set("vdp_scan_step_from", vdp.scanner.step_from)
        .set("vdp_scan_stepped_ago", vdp.scanner.stepped_ago)
        .set(
            "vdp_scan_field_hold",
            match vdp.scanner.field_hold {
                Some(held) => StateValue::from(held),
                None => StateValue::Null,
            },
        )
        .set("vdp_scan_fifth_match", vdp.scanner.fifth_match_this_scan)
        .set("vdp_segment_bits", vdp.segment.bits)
        .set("vdp_segment_foreground", vdp.segment.foreground)
        .set("vdp_segment_background", vdp.segment.background)
        .set("vdp_segment_start_x", vdp.segment.start_x)
        .set("vdp_segment_end_x", vdp.segment.end_x);
}

fn write_psg(r: &mut StateRecord, psg: &PsgState) {
    for (index, (period, counter, output, attenuation)) in TONE_FIELDS.into_iter().enumerate() {
        r.set(period, psg.tones[index].period)
            .set(counter, psg.tones[index].counter)
            .set(output, psg.tones[index].output)
            .set(attenuation, psg.attenuations[index]);
    }
    r.set("psg_noise_attenuation", psg.attenuations[3])
        .set(
            "psg_noise_control",
            psg.noise.mode.bits() | psg.noise.rate.bits(),
        )
        .set(
            "psg_latched_register",
            latched_register(psg.latched_channel, psg.latched_kind),
        )
        .set("psg_noise_counter", psg.noise.counter)
        .set("psg_noise_output", psg.noise.output)
        .set("psg_noise_shift_register", psg.noise.shift_register)
        .set("psg_clock_divider", psg.clock_divider)
        .set("psg_ready_countdown", psg.ready_countdown);
}

fn write_board(r: &mut StateRecord, board: &BoardState) {
    r.set("joystick_dc", board.joystick_dc)
        .set("joystick_dd", board.joystick_dd)
        .set("audio_sample_phase", board.sample_phase)
        .set("fields_taken", board.fields_taken as u32);
}

// ── Restore ───────────────────────────────────────────────────────

/// Rebuild the console in place from a validated record and its byte regions,
/// at an instruction boundary. Errors (never panics) on a malformed record.
pub fn restore(
    sg: &mut Sg1000,
    record: &StateRecord,
    memory: &[(String, Vec<u8>)],
    frame: Option<&StateFrame>,
) -> Result<(), StateError> {
    // Parse the whole record before mutating anything, so a malformed field
    // leaves the console untouched rather than half-restored.
    let cpu = parse_cpu(record)?;
    let vdp = parse_vdp(record)?;
    let psg = parse_psg(record)?;
    let board = parse_board(record)?;

    sg.cpu.restore_boundary(&cpu);
    sg.vdp_mut().restore_boundary(&vdp);
    sg.psg_mut().restore_boundary(&psg);
    sg.restore_board(&board);

    let region = |name: &str| {
        memory
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, bytes)| bytes.as_slice())
    };
    if let Some(bytes) = region("work_ram") {
        sg.restore_work_ram(bytes);
    }
    if let Some(bytes) = region("vram") {
        sg.vdp_mut().restore_vram(bytes);
    }
    if let Some(bytes) = region("vdp_line") {
        sg.vdp_mut().restore_line_buffer(bytes);
    }
    if let Some(bytes) = region("vdp_sprite_plane") {
        sg.vdp_mut().restore_sprite_plane(bytes);
    }
    if let Some(frame) = frame {
        sg.vdp_mut().restore_raster(&frame.data);
    }
    Ok(())
}

fn parse_cpu(r: &StateRecord) -> Result<CpuState, StateError> {
    Ok(CpuState {
        a: u8_of(r, "a")?,
        f: u8_of(r, "f")?,
        b: u8_of(r, "b")?,
        c: u8_of(r, "c")?,
        d: u8_of(r, "d")?,
        e: u8_of(r, "e")?,
        h: u8_of(r, "h")?,
        l: u8_of(r, "l")?,
        a_alt: u8_of(r, "a_alt")?,
        f_alt: u8_of(r, "f_alt")?,
        b_alt: u8_of(r, "b_alt")?,
        c_alt: u8_of(r, "c_alt")?,
        d_alt: u8_of(r, "d_alt")?,
        e_alt: u8_of(r, "e_alt")?,
        h_alt: u8_of(r, "h_alt")?,
        l_alt: u8_of(r, "l_alt")?,
        ix: u16_of(r, "ix")?,
        iy: u16_of(r, "iy")?,
        sp: u16_of(r, "sp")?,
        pc: u16_of(r, "pc")?,
        wz: u16_of(r, "wz")?,
        i: u8_of(r, "i")?,
        r: u8_of(r, "r")?,
        iff1: bool_of(r, "iff1")?,
        iff2: bool_of(r, "iff2")?,
        interrupt_mode: interrupt_mode(u8_of(r, "im")?)?,
        halted: bool_of(r, "halted")?,
        ei_pending: bool_of(r, "ei_pending")?,
        q: u8_of(r, "q")?,
        flags_touched: bool_of(r, "flags_touched")?,
        p: bool_of(r, "p")?,
        nmi_pending: bool_of(r, "nmi_pending")?,
        irq_line: bool_of(r, "irq_line")?,
        irq_sampled: bool_of(r, "irq_sampled")?,
        address_bus: u16_of(r, "address_bus")?,
    })
}

fn parse_vdp(r: &StateRecord) -> Result<VdpState, StateError> {
    let mut registers = [0u8; 8];
    for (index, name) in REGISTER_FIELDS.into_iter().enumerate() {
        registers[index] = u8_of(r, name)?;
    }
    Ok(VdpState {
        registers,
        line: u16_of(r, "vdp_line")?,
        line_xtal: u16_of(r, "vdp_line_xtal")? as u32,
        fields_completed: u32_of(r, "vdp_fields_completed")? as u64,
        port: PortState {
            address: u16_of(r, "vdp_address")?,
            awaiting_second_byte: bool_of(r, "vdp_awaiting_second_byte")?,
            read_buffer: u8_of(r, "vdp_read_buffer")?,
            transfer: transfer(
                bool_of(r, "vdp_transfer_write")?,
                u8_of(r, "vdp_transfer_data")?,
            ),
            prior_transfer: transfer(
                bool_of(r, "vdp_prior_transfer_write")?,
                u8_of(r, "vdp_prior_transfer_data")?,
            ),
            transfer_written_ago: u32_of(r, "vdp_transfer_written_ago")?,
            pending_address: u16_of(r, "vdp_pending_address")?,
            pending_flag: bool_of(r, "vdp_pending_flag")?,
            access: bool_of(r, "vdp_access_active")?.then_some(AccessState {
                address: u16_of(r, "vdp_access_address")?,
                claimed_ago: u8_of(r, "vdp_access_claimed_ago")?,
            }),
        },
        status: StatusState {
            frame: bool_of(r, "vdp_frame_flag")?,
            fifth_sprite: bool_of(r, "vdp_fifth_sprite_flag")?,
            coincidence: bool_of(r, "vdp_coincidence_flag")?,
            frame_set_ago: u32_of(r, "vdp_frame_flag_set_ago")?,
            fifth_sprite_set_ago: u32_of(r, "vdp_fifth_sprite_set_ago")?,
            sprite_field: u8_of(r, "vdp_fifth_sprite_index")?,
        },
        scanner: ScannerState {
            counter: u8_of(r, "vdp_scan_counter")?,
            stop: scan_stop(
                u8_of(r, "vdp_scan_stop_kind")?,
                u8_of(r, "vdp_scan_stop_index")?,
            )?,
            stepped_ago: u32_of(r, "vdp_scan_stepped_ago")?,
            step_from: u8_of(r, "vdp_scan_step_from")?,
            field_hold: opt_u8(r, "vdp_scan_field_hold")?,
            fifth_match_this_scan: bool_of(r, "vdp_scan_fifth_match")?,
        },
        segment: SegmentState {
            bits: u8_of(r, "vdp_segment_bits")?,
            foreground: u8_of(r, "vdp_segment_foreground")?,
            background: u8_of(r, "vdp_segment_background")?,
            start_x: u16_of(r, "vdp_segment_start_x")?,
            end_x: u16_of(r, "vdp_segment_end_x")?,
        },
    })
}

fn parse_psg(r: &StateRecord) -> Result<PsgState, StateError> {
    let mut tones = [ToneState {
        period: 0,
        counter: 0,
        output: true,
    }; 3];
    let mut attenuations = [0u8; 4];
    for (index, (period, counter, output, attenuation)) in TONE_FIELDS.into_iter().enumerate() {
        tones[index] = ToneState {
            period: u16_of(r, period)?,
            counter: u16_of(r, counter)?,
            output: bool_of(r, output)?,
        };
        attenuations[index] = u8_of(r, attenuation)?;
    }
    attenuations[3] = u8_of(r, "psg_noise_attenuation")?;
    let control = u8_of(r, "psg_noise_control")?;
    let latched = u8_of(r, "psg_latched_register")?;
    Ok(PsgState {
        latched_channel: Channel::ALL[(latched as usize >> 1) & 0x03],
        latched_kind: match latched & 1 {
            0 => RegisterKind::Frequency,
            _ => RegisterKind::Attenuation,
        },
        tones,
        noise: NoiseState {
            rate: NoiseRate::from_control(control),
            mode: NoiseMode::from_control(control),
            counter: u16_of(r, "psg_noise_counter")?,
            output: bool_of(r, "psg_noise_output")?,
            shift_register: u16_of(r, "psg_noise_shift_register")?,
        },
        attenuations,
        clock_divider: u8_of(r, "psg_clock_divider")?,
        ready_countdown: u8_of(r, "psg_ready_countdown")?,
    })
}

fn parse_board(r: &StateRecord) -> Result<BoardState, StateError> {
    Ok(BoardState {
        joystick_dc: u8_of(r, "joystick_dc")?,
        joystick_dd: u8_of(r, "joystick_dd")?,
        sample_phase: match r.get("audio_sample_phase") {
            Some(StateValue::Int(phase)) => *phase,
            Some(StateValue::Null) | None => 0,
            _ => return Err(StateError::Corrupt),
        },
        fields_taken: u32_of(r, "fields_taken")? as u64,
    })
}

// ── Field names and encodings ─────────────────────────────────────

const REGISTER_FIELDS: [&str; 8] = [
    "vdp_r0", "vdp_r1", "vdp_r2", "vdp_r3", "vdp_r4", "vdp_r5", "vdp_r6", "vdp_r7",
];

/// Per tone generator: period register, counter, flip-flop, attenuation.
const TONE_FIELDS: [(&str, &str, &str, &str); 3] = [
    (
        "psg_tone1_period",
        "psg_tone1_counter",
        "psg_tone1_output",
        "psg_tone1_attenuation",
    ),
    (
        "psg_tone2_period",
        "psg_tone2_counter",
        "psg_tone2_output",
        "psg_tone2_attenuation",
    ),
    (
        "psg_tone3_period",
        "psg_tone3_counter",
        "psg_tone3_output",
        "psg_tone3_attenuation",
    ),
];

fn interrupt_mode_number(mode: InterruptMode) -> u8 {
    match mode {
        InterruptMode::Mode0 => 0,
        InterruptMode::Mode1 => 1,
        InterruptMode::Mode2 => 2,
    }
}

fn interrupt_mode(number: u8) -> Result<InterruptMode, StateError> {
    match number {
        0 => Ok(InterruptMode::Mode0),
        1 => Ok(InterruptMode::Mode1),
        2 => Ok(InterruptMode::Mode2),
        _ => Err(StateError::Corrupt),
    }
}

fn transfer_parts(transfer: PortTransfer) -> (bool, u8) {
    match transfer {
        PortTransfer::Write(value) => (true, value),
        PortTransfer::Refill => (false, 0),
    }
}

fn transfer(is_write: bool, data: u8) -> PortTransfer {
    match is_write {
        true => PortTransfer::Write(data),
        false => PortTransfer::Refill,
    }
}

fn stop_parts(stop: ScanStop) -> (u8, u8) {
    match stop {
        ScanStop::FullWalk => (0, 0),
        ScanStop::Terminator(index) => (1, index),
        ScanStop::FifthMatch(index) => (2, index),
    }
}

fn scan_stop(kind: u8, index: u8) -> Result<ScanStop, StateError> {
    match kind {
        0 => Ok(ScanStop::FullWalk),
        1 => Ok(ScanStop::Terminator(index)),
        2 => Ok(ScanStop::FifthMatch(index)),
        _ => Err(StateError::Corrupt),
    }
}

fn latched_register(channel: Channel, kind: RegisterKind) -> u8 {
    let kind = match kind {
        RegisterKind::Frequency => 0,
        RegisterKind::Attenuation => 1,
    };
    (channel.index() as u8) << 1 | kind
}

fn u8_of(r: &StateRecord, name: &str) -> Result<u8, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u8::MAX as u32 => Ok(*value as u8),
        _ => Err(StateError::Corrupt),
    }
}

fn u16_of(r: &StateRecord, name: &str) -> Result<u16, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u16::MAX as u32 => Ok(*value as u16),
        _ => Err(StateError::Corrupt),
    }
}

fn u32_of(r: &StateRecord, name: &str) -> Result<u32, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}

fn bool_of(r: &StateRecord, name: &str) -> Result<bool, StateError> {
    match r.get(name) {
        Some(StateValue::Bool(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}

/// A nullable u8 field: `None` when the record carries it as null.
fn opt_u8(r: &StateRecord, name: &str) -> Result<Option<u8>, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u8::MAX as u32 => Ok(Some(*value as u8)),
        Some(StateValue::Null) => Ok(None),
        _ => Err(StateError::Corrupt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_schema::sg1000_state_schema;

    /// A powered-on board's record carries every field the schema names.
    #[test]
    fn a_captured_record_validates_against_the_schema() {
        let mut console = Sg1000::new(&[0; 0x2000]).expect("flat cartridge image");
        for _ in 0..64 {
            console.step_instruction();
        }
        let record = read_state(&console).expect("a boundary record");
        assert_eq!(record.validate(sg1000_state_schema()), Ok(()));
    }

    #[test]
    fn a_capture_is_refused_mid_instruction() {
        let mut console = Sg1000::new(&[0; 0x2000]).expect("flat cartridge image");
        console.step_tstate();
        assert!(!console.at_instruction_boundary());
        assert!(matches!(capture(&console), Err(StateError::NotAtBoundary)));
    }
}
