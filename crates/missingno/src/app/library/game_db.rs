//! Per-game metadata database.
//!
//! Contains information about specific games that the emulator uses to
//! improve behaviour — currently just SRAM regions to ignore when
//! detecting save changes.

use std::ops::Range;

/// Regions of SRAM that should be ignored when detecting save changes.
/// These are scratch areas that games write to frequently during normal
/// gameplay without the player actually saving.
pub fn ignored_sram_regions(title: &str) -> &'static [Range<usize>] {
    match title {
        // Pokemon Gen 1: sprite decompression buffers in SRAM bank 0
        // Three 0x188-byte buffers at 0x0000, 0x0188, 0x0310
        "POKEMON RED" | "POKEMON BLUE" | "POKEMON GREEN" | "POKEMON YELLOW" => &[0x0000..0x0498],
        _ => &[],
    }
}

/// Check whether SRAM has meaningfully changed compared to a previous
/// snapshot, ignoring scratch regions for the given game.
pub fn sram_changed(title: &str, current: &[u8], previous: &[u8]) -> bool {
    if current.len() != previous.len() {
        return true;
    }

    let ignored = ignored_sram_regions(title);

    for (i, (a, b)) in current.iter().zip(previous.iter()).enumerate() {
        if a != b && !ignored.iter().any(|r| r.contains(&i)) {
            return true;
        }
    }

    false
}
