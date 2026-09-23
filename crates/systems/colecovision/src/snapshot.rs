//! The ColecoVision save-state bridge: the three chips through their own
//! record bridges, and the board's latches and switches beside them. A capture
//! is refused anywhere but an instruction boundary; the VDP and the PSG are
//! captured wherever that boundary lands them, and the rows the field has
//! already emitted travel as the state's framebuffer.

use missingno_core::machine::BoundaryState;
use missingno_core::state::{PixelFormat, StateRecord, StateValue};
use missingno_core::state_file::StateFrame;
use missingno_core::system::StateError;

use crate::console::{BoardState, ColecoVision};
use crate::controllers::{ControllerMode, HandController};

/// Read the whole board into a schema-keyed record; `None` mid-instruction.
pub fn read_state(cv: &ColecoVision) -> Option<StateRecord> {
    let cpu = cv.cpu.boundary_state()?;
    let mut record = StateRecord::new();
    missingno_zilog_z80::record::write_state(&mut record, &cpu);
    missingno_ti_vdp::record::write_state(&mut record, &cv.vdp().boundary_state());
    missingno_ti_psg::record::write_state(&mut record, &cv.psg().boundary_state());
    write_board(&mut record, &cv.board_state());
    Some(record)
}

/// The whole boundary state a save file carries.
pub fn capture(cv: &ColecoVision) -> Result<BoundaryState, StateError> {
    let record = read_state(cv).ok_or(StateError::NotAtBoundary)?;
    Ok(BoundaryState {
        record,
        memory: vec![
            ("ram", cv.ram().to_vec()),
            ("vram", cv.vdp().vram().to_vec()),
            ("vdp_line", cv.vdp().line_buffer().to_vec()),
            ("vdp_sprite_plane", cv.vdp().sprite_plane().to_vec()),
        ],
        frame: Some(raster(cv)),
    })
}

/// The field being emitted, as the state file's framebuffer carries it.
fn raster(cv: &ColecoVision) -> StateFrame {
    let frame = cv.vdp().frame();
    StateFrame {
        width: frame.width as u32,
        height: Some(frame.height as u32),
        format: PixelFormat::Indexed8,
        data: frame.pixels.clone(),
    }
}

// ── Capture ───────────────────────────────────────────────────────

const LEFT_FIRE: u8 = 0x01;
const RIGHT_FIRE: u8 = 0x02;

/// Each connector's fields, J5 then J6: stick lines, buttons, keys.
const CONTROLLER_FIELDS: [[&str; 3]; 2] = [
    ["pad_lines_1", "pad_buttons_1", "keys_1"],
    ["pad_lines_2", "pad_buttons_2", "keys_2"],
];

fn write_board(r: &mut StateRecord, board: &BoardState) {
    r.set("controller_mode", board.mode == ControllerMode::Keypad)
        .set("wait_latch_q", board.wait_latch_q);
    for (pad, [lines, buttons, keys]) in board.controllers.iter().zip(CONTROLLER_FIELDS) {
        let held =
            if pad.left_fire { LEFT_FIRE } else { 0 } | if pad.right_fire { RIGHT_FIRE } else { 0 };
        r.set(lines, pad.lines)
            .set(buttons, held)
            .set(keys, pad.keys);
    }
    r.set("audio_sample_phase", board.sample_phase)
        .set("fields_taken", board.fields_taken as u32);
}

// ── Restore ───────────────────────────────────────────────────────

/// Rebuild the console in place from a validated record and its byte regions,
/// at an instruction boundary. Errors (never panics) on a malformed record.
pub fn restore(
    cv: &mut ColecoVision,
    record: &StateRecord,
    memory: &[(String, Vec<u8>)],
    frame: Option<&StateFrame>,
) -> Result<(), StateError> {
    // Parse the whole record before mutating anything.
    let cpu = missingno_zilog_z80::record::parse_state(record)?;
    let vdp = missingno_ti_vdp::record::parse_state(record)?;
    let psg = missingno_ti_psg::record::parse_state(record)?;
    let board = parse_board(record)?;

    cv.cpu.restore_boundary(&cpu);
    cv.vdp_mut().restore_boundary(&vdp);
    cv.psg_mut().restore_boundary(&psg);
    cv.restore_board(&board);

    let region = |name: &str| {
        memory
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, bytes)| bytes.as_slice())
    };
    if let Some(bytes) = region("ram") {
        cv.restore_ram(bytes);
    }
    if let Some(bytes) = region("vram") {
        cv.vdp_mut().restore_vram(bytes);
    }
    if let Some(bytes) = region("vdp_line") {
        cv.vdp_mut().restore_line_buffer(bytes);
    }
    if let Some(bytes) = region("vdp_sprite_plane") {
        cv.vdp_mut().restore_sprite_plane(bytes);
    }
    if let Some(frame) = frame {
        cv.vdp_mut().restore_raster(&frame.data);
    }
    Ok(())
}

fn parse_board(r: &StateRecord) -> Result<BoardState, StateError> {
    let controller = |[lines, buttons, keys]: [&str; 3]| -> Result<HandController, StateError> {
        let held = int_of(r, buttons, u8::MAX as u32)? as u8;
        Ok(HandController {
            lines: int_of(r, lines, u8::MAX as u32)? as u8,
            left_fire: held & LEFT_FIRE != 0,
            right_fire: held & RIGHT_FIRE != 0,
            keys: int_of(r, keys, u16::MAX as u32)? as u16,
        })
    };
    Ok(BoardState {
        mode: match bool_of(r, "controller_mode")? {
            true => ControllerMode::Keypad,
            false => ControllerMode::Joystick,
        },
        wait_latch_q: bool_of(r, "wait_latch_q")?,
        controllers: [
            controller(CONTROLLER_FIELDS[0])?,
            controller(CONTROLLER_FIELDS[1])?,
        ],
        sample_phase: match r.get("audio_sample_phase") {
            Some(StateValue::Int(phase)) => *phase,
            Some(StateValue::Null) | None => 0,
            _ => return Err(StateError::Corrupt),
        },
        fields_taken: int_of(r, "fields_taken", u32::MAX)? as u64,
    })
}

fn int_of(r: &StateRecord, name: &str, max: u32) -> Result<u32, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= max => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}

fn bool_of(r: &StateRecord, name: &str) -> Result<bool, StateError> {
    match r.get(name) {
        Some(StateValue::Bool(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::BIOS_SIZE;
    use crate::state_schema::colecovision_state_schema;
    use missingno_core::ports::PortId;
    use missingno_core::system::{ControlId, ControlInput, ControlRole};
    use missingno_ti_vdp::Standard;

    fn console() -> ColecoVision {
        ColecoVision::new(&[0; 0x2000], Standard::Ntsc, [0; BIOS_SIZE]).expect("a flat image")
    }

    #[test]
    fn a_captured_record_validates_against_the_schema() {
        let mut console = console();
        for _ in 0..64 {
            console.step_instruction();
        }
        let record = read_state(&console).expect("a boundary record");
        assert_eq!(record.validate(colecovision_state_schema()), Ok(()));
    }

    #[test]
    fn a_capture_is_refused_mid_instruction() {
        let mut console = console();
        console.step_tstate();
        assert!(!console.at_instruction_boundary());
        assert!(matches!(capture(&console), Err(StateError::NotAtBoundary)));
    }

    #[test]
    fn the_board_state_round_trips_through_a_record() {
        let mut original = console();
        for (port, role) in [
            (PortId(0), ControlRole::Up),
            (PortId(0), ControlRole::Action(1)),
            (PortId(1), ControlRole::Key(11)),
        ] {
            original.apply_control(ControlId::port(port, role), ControlInput::Digital(true));
        }
        let state = capture(&original).expect("a boundary save");
        let mut restored = console();
        let memory: Vec<(String, Vec<u8>)> = state
            .memory
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), bytes.clone()))
            .collect();
        restore(&mut restored, &state.record, &memory, state.frame.as_ref())
            .expect("restore succeeds");
        assert_eq!(restored.board_state(), original.board_state());
    }
}
