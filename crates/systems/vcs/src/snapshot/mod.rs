//! The VCS save-state bridge: it maps the console's internal state onto the
//! hardware-named [`SystemStateSchema`] and back. Capture reads the console into
//! a [`StateRecord`] keyed by the schema's field names; restore parses a record
//! and rebuilds the console in place at an instruction boundary.
//!
//! At a VCS instruction boundary the CPU is at a fetch boundary (no
//! micro-sequencer residue) and the φ0 grid phase follows the captured beam
//! position, so there is no Tier-2b residue — the whole die state is captured
//! and the restore is bit-exact for every scanline emitted after it. The
//! frame-assembly buffers and audio resampler window are the frontend
//! Television's off-chip integration surface and are reconstructed empty; the
//! field re-locks on the next VSYNC.

mod capture;
mod fields;
mod restore;

pub use capture::{capture_memory, read_state};
pub use restore::restore;
