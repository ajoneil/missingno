//! Save-state glue shared by both seam halves: the ROM fingerprint a state is
//! bound to, and the read/write pair around the schema-keyed state file.

use missingno_core::state::PixelFormat;
use missingno_core::state_file::{StateFrame, StateMeta, read_state_file, write_state_file};
use missingno_core::system::StateError;
use missingno_core::video::IndexedFrame;

use crate::console::Vcs;
use crate::state_schema::vcs_state_schema;

/// A hex SHA-256 of the raw ROM image, taken at load (the cartridge does not
/// retain a plain board's image), so a save state can refuse a ROM it was not
/// written for.
pub(super) fn rom_fingerprint(rom: &[u8]) -> String {
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
pub(super) fn save_state_bytes(
    vcs: &Vcs,
    frame: &IndexedFrame,
    rom_sha256: &str,
) -> Option<Vec<u8>> {
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
pub(super) fn load_state_into(
    vcs: &mut Vcs,
    bytes: &[u8],
    rom_sha256: &str,
) -> Result<(), StateError> {
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
