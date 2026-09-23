//! The machine seam's trace capture over a corpus ROM.

#![cfg(feature = "morepork")]

use missingno_core::machine::Machine;
use missingno_sg1000::console::Sg1000;
use missingno_sg1000::debug::Sg1000System;
use missingno_ti_vdp::Standard;

const SANITY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../chips/ti-vdp/tests/accuracy/roms/harness/sanity.sg"
);

#[test]
fn a_capture_writes_a_trace_and_returns_its_frame() {
    let image = std::fs::read(SANITY).expect("the corpus sanity ROM");
    let mut console = Sg1000::new(&image, None, Standard::Ntsc).expect("flat cartridge image");
    let path = std::env::temp_dir().join(format!("sg1000-capture-{}.morepork", std::process::id()));

    let frame = Sg1000System::capture_trace(&mut console, &path);
    let written = std::fs::metadata(&path).map(|meta| meta.len());
    let _ = std::fs::remove_file(&path);

    assert!(frame.is_some(), "no frame completed within the budget");
    assert!(written.expect("the trace file") > 0);
}
