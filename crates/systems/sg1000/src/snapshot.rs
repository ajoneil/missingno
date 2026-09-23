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

use crate::console::{BoardState, Sg1000};

/// Read the whole board into a schema-keyed record; `None` mid-instruction,
/// where the CPU carries residue no field names.
pub fn read_state(sg: &Sg1000) -> Option<StateRecord> {
    let cpu = sg.cpu.boundary_state()?;
    let vdp = sg.vdp().boundary_state();
    let psg = sg.psg().boundary_state();
    let board = sg.board_state();

    let mut record = StateRecord::new();
    missingno_zilog_z80::record::write_state(&mut record, &cpu);
    missingno_ti_vdp::record::write_state(&mut record, &vdp);
    missingno_ti_psg::record::write_state(&mut record, &psg);
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

/// The byte regions beside the record: the work RAM, a RAM-bearing cartridge's
/// own stores, the VDP's DRAM, and the two line buffers under the raster.
fn capture_memory(sg: &Sg1000) -> Vec<(&'static str, Vec<u8>)> {
    let mut regions = vec![
        ("work_ram", sg.work_ram().to_vec()),
        ("vram", sg.vdp().vram().to_vec()),
        ("vdp_line", sg.vdp().line_buffer().to_vec()),
        ("vdp_sprite_plane", sg.vdp().sprite_plane().to_vec()),
    ];
    if let Some(ram) = sg.cart_ram() {
        regions.push(("cart_ram", ram));
    }
    regions
}

/// The field being emitted, as the state file's framebuffer carries it.
fn raster(sg: &Sg1000) -> StateFrame {
    let frame = sg.vdp().frame();
    StateFrame {
        width: frame.width as u32,
        height: Some(frame.height as u32),
        format: PixelFormat::Indexed8,
        data: frame.pixels.clone(),
    }
}

// ── Capture ───────────────────────────────────────────────────────

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
    let cpu = missingno_zilog_z80::record::parse_state(record)?;
    let vdp = missingno_ti_vdp::record::parse_state(record)?;
    let psg = missingno_ti_psg::record::parse_state(record)?;
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
    if let Some(bytes) = region("cart_ram") {
        sg.restore_cart_ram(bytes);
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

fn u8_of(r: &StateRecord, name: &str) -> Result<u8, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u8::MAX as u32 => Ok(*value as u8),
        _ => Err(StateError::Corrupt),
    }
}

fn u32_of(r: &StateRecord, name: &str) -> Result<u32, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_schema::sg1000_state_schema;
    use missingno_ti_vdp::Standard;

    /// A powered-on board's record carries every field the schema names.
    #[test]
    fn a_captured_record_validates_against_the_schema() {
        let mut console =
            Sg1000::new(&[0; 0x2000], None, Standard::Ntsc).expect("flat cartridge image");
        for _ in 0..64 {
            console.step_instruction();
        }
        let record = read_state(&console).expect("a boundary record");
        assert_eq!(record.validate(sg1000_state_schema()), Ok(()));
    }

    #[test]
    fn a_capture_is_refused_mid_instruction() {
        let mut console =
            Sg1000::new(&[0; 0x2000], None, Standard::Ntsc).expect("flat cartridge image");
        console.step_tstate();
        assert!(!console.at_instruction_boundary());
        assert!(matches!(capture(&console), Err(StateError::NotAtBoundary)));
    }
}
