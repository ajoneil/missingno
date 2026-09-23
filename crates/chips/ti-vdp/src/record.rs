//! The VDP's state in the hardware-named state vocabulary: the fields and byte
//! regions a board's schema carries for the part, and the capture and restore
//! through a [`StateRecord`] keyed on them.
//!
//! The chip's quantum is finer than any CPU's, so it is captured whole — its
//! counters and in-flight access are live wherever a boundary falls.

use missingno_core::state::{FieldDef, FieldType, MemorySpan, StateRecord, StateValue};
use missingno_core::system::StateError;

use crate::{
    AccessState, PortState, PortTransfer, ScanStop, ScannerState, SegmentState, StatusState,
    VISIBLE_WIDTH, VRAM_SIZE, VdpState,
};

use FieldType::{Bool, U8, U16, U32};

/// The line-latched sprite plane covers the display area only.
const SPRITE_PLANE_WIDTH: u32 = 256;

/// The register file, pointer and status flags as observable fields, then the
/// counters and engines a bit-exact restore also needs.
pub fn state_fields() -> Vec<FieldDef> {
    let mut fields = Vec::new();
    for (index, register) in REGISTER_FIELDS.into_iter().enumerate() {
        fields.push(FieldDef::observable(register, U8, "vdp").help(match index {
            0 => "mode bits M3 and the external-video enable",
            1 => "RAM size, display and interrupt enables, M1/M2, sprite size and MAG",
            2 => "name table base",
            3 => "colour table base",
            4 => "pattern generator base",
            5 => "sprite attribute table base",
            6 => "sprite pattern generator base",
            _ => "text colour and backdrop",
        }));
    }
    fields.push(
        FieldDef::observable("vdp_address", U16, "vdp").help("the auto-incrementing VRAM pointer"),
    );
    fields.push(FieldDef::observable("vdp_frame_flag", Bool, "vdp").help("status F"));
    fields.push(FieldDef::observable("vdp_fifth_sprite_flag", Bool, "vdp").help("status 5S"));
    fields.push(FieldDef::observable("vdp_coincidence_flag", Bool, "vdp").help("status C"));
    fields.push(
        FieldDef::observable("vdp_fifth_sprite_index", U8, "vdp")
            .help("the SAT index latched with 5S"),
    );

    fields.extend([
        FieldDef::boundary("vdp_line", U16, "vdp").help("the vertical counter"),
        FieldDef::boundary("vdp_line_xtal", U16, "vdp")
            .help("XTAL periods elapsed within the line"),
        FieldDef::boundary("vdp_fields_completed", U32, "vdp")
            .help("visible rasters completed since power-on")
            .sourced("missingno"),
        FieldDef::boundary("vdp_awaiting_second_byte", Bool, "vdp")
            .help("the control port holds a first byte"),
        FieldDef::boundary("vdp_read_buffer", U8, "vdp").help("the read-ahead buffer"),
        FieldDef::boundary("vdp_transfer_write", Bool, "vdp")
            .help("the latched transfer is a write, not a read-ahead refill"),
        FieldDef::boundary("vdp_transfer_data", U8, "vdp").help("the latched transfer's byte"),
        FieldDef::boundary("vdp_prior_transfer_write", Bool, "vdp")
            .help("the transfer this one replaced"),
        FieldDef::boundary("vdp_prior_transfer_data", U8, "vdp"),
        FieldDef::boundary("vdp_transfer_written_ago", U32, "vdp")
            .help("XTAL periods since the transfer register was written"),
        FieldDef::boundary("vdp_pending_address", U16, "vdp")
            .help("the address latched by the request that raised the flag"),
        FieldDef::boundary("vdp_pending_flag", Bool, "vdp").help("a CPU access is waiting"),
        FieldDef::boundary("vdp_access_active", Bool, "vdp")
            .help("a memory cycle has claimed the access"),
        FieldDef::boundary("vdp_access_address", U16, "vdp"),
        FieldDef::boundary("vdp_access_claimed_ago", U8, "vdp")
            .help("XTAL periods since the claim; the lock and release follow"),
        FieldDef::boundary("vdp_frame_flag_set_ago", U32, "vdp")
            .help("XTAL periods since F was set, which a read's strobe races"),
        FieldDef::boundary("vdp_fifth_sprite_set_ago", U32, "vdp")
            .help("XTAL periods since 5S was set"),
        FieldDef::boundary("vdp_scan_counter", U8, "vdp")
            .help("the sprite pre-processing scanner's SAT index"),
        FieldDef::boundary("vdp_scan_stop_kind", U8, "vdp")
            .help("where this line's ramp ends: 0 full walk / 1 terminator / 2 fifth match"),
        FieldDef::boundary("vdp_scan_stop_index", U8, "vdp").help("the SAT index it stops at"),
        FieldDef::boundary("vdp_scan_step_from", U8, "vdp")
            .help("the counter value the latest step replaced"),
        FieldDef::boundary("vdp_scan_stepped_ago", U32, "vdp")
            .help("XTAL periods since that step, which the presented field window rides"),
        FieldDef::boundary("vdp_segment_bits", U8, "vdp").help("the latched fetch's pattern byte"),
        FieldDef::boundary("vdp_segment_foreground", U8, "vdp"),
        FieldDef::boundary("vdp_segment_background", U8, "vdp"),
        FieldDef::boundary("vdp_segment_start_x", U16, "vdp"),
        FieldDef::boundary("vdp_segment_end_x", U16, "vdp"),
    ]);

    fields
}

/// The chip's DRAM and the two line buffers under the raster.
pub fn memory_spans() -> Vec<MemorySpan> {
    vec![
        MemorySpan::off_bus("vram", VRAM_SIZE as u32).help("the VDP's DRAM, in physical order"),
        MemorySpan::off_bus("vdp_line", VISIBLE_WIDTH as u32)
            .help("the row being composited under the raster"),
        MemorySpan::off_bus("vdp_sprite_plane", SPRITE_PLANE_WIDTH)
            .help("the line-latched sprite plane of the row being emitted"),
    ]
}

pub fn write_state(r: &mut StateRecord, vdp: &VdpState) {
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
        .set("vdp_segment_bits", vdp.segment.bits)
        .set("vdp_segment_foreground", vdp.segment.foreground)
        .set("vdp_segment_background", vdp.segment.background)
        .set("vdp_segment_start_x", vdp.segment.start_x)
        .set("vdp_segment_end_x", vdp.segment.end_x);
}

pub fn parse_state(r: &StateRecord) -> Result<VdpState, StateError> {
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

const REGISTER_FIELDS: [&str; 8] = [
    "vdp_r0", "vdp_r1", "vdp_r2", "vdp_r3", "vdp_r4", "vdp_r5", "vdp_r6", "vdp_r7",
];

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
