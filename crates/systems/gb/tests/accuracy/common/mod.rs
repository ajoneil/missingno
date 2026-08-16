pub use missingno_gb::test_support::*;
pub use missingno_test_support::compare::{assert_pixels_match, hex_byte};

/// Load a reference PNG from the shared roms dir as one shade byte per pixel.
pub fn load_reference_png(relative: &str) -> Vec<u8> {
    missingno_test_support::reference::ReferencePng::load(&rom_path(relative)).greyscale()
}
