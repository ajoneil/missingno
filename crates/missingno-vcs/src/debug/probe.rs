//! Reading a ROM's hardware assumptions when the library's metadata is silent:
//! which broadcast standard its kernel was written for, and which board a
//! game-db code names.

use missingno_core::video::{self, Television};

use crate::console::Vcs;
use crate::tia::VISIBLE_CLOCKS;
use crate::{CartType, TvStandard};

use super::frame::{FRAME_BUDGET_LINES, VSYNC_LOCK_LINES};

/// Scanlines per field that split NTSC (~262) from PAL (~312); the midpoint
/// clears the ~284–290 overlap where a handful of ROMs are genuinely ambiguous.
const NTSC_PAL_FIELD_THRESHOLD: usize = 287;

/// Detect an uncatalogued ROM's broadcast standard by counting scanlines per
/// field: a PAL field runs ~50 lines longer than NTSC. The standard only scales
/// the master clock, not the kernel's line count, so a provisional NTSC build
/// reads the field length truthfully.
pub(super) fn probe_tv_standard(rom: &[u8], cart_type: Option<CartType>) -> TvStandard {
    let Ok(mut vcs) = Vcs::new(rom, TvStandard::Ntsc, cart_type) else {
        return TvStandard::Ntsc;
    };
    let mut tv = Television::<VISIBLE_CLOCKS>::new(VSYNC_LOCK_LINES);
    let mut fields = Vec::new();
    let mut lines_this_field = 0usize;
    // A few fields, bounded so a kernel that never syncs can't spin.
    for _ in 0..(FRAME_BUDGET_LINES * 8) {
        let line = vcs.step_scanline();
        lines_this_field += 1;
        if tv
            .feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            })
            .is_some()
        {
            fields.push(lines_this_field);
            lines_this_field = 0;
            if fields.len() >= 6 {
                break;
            }
        }
    }
    classify_fields(&fields)
}

/// Classify measured field lengths by their median (robust to a long startup
/// field), skipping the first warm-up field; NTSC when nothing synced.
fn classify_fields(fields: &[usize]) -> TvStandard {
    let mut steady: Vec<usize> = fields.iter().copied().skip(1).collect();
    if steady.is_empty() {
        return TvStandard::Ntsc;
    }
    steady.sort_unstable();
    if steady[steady.len() / 2] > NTSC_PAL_FIELD_THRESHOLD {
        TvStandard::Pal
    } else {
        TvStandard::Ntsc
    }
}

/// Parse a game-db board code into the core's board type; codes the core can't
/// build yet return `None`, leaving `Cartridge::load` to size-detect.
pub(super) fn core_cart_type(code: &str) -> Option<CartType> {
    match code {
        "2K" => Some(CartType::Plain2K),
        "4K" => Some(CartType::Plain4K),
        "F8" => Some(CartType::F8),
        "F8SC" => Some(CartType::F8Sc),
        "F6" => Some(CartType::F6),
        "F6SC" => Some(CartType::F6Sc),
        "F4" => Some(CartType::F4),
        "F4SC" => Some(CartType::F4Sc),
        "FA" => Some(CartType::Fa),
        "FC" => Some(CartType::Fc),
        "FE" => Some(CartType::Fe),
        "E0" => Some(CartType::E0),
        "E7" => Some(CartType::E7),
        "CV" => Some(CartType::Cv),
        "UA" => Some(CartType::Ua),
        "3F" => Some(CartType::ThreeF),
        "3E" => Some(CartType::ThreeE),
        "3E+" => Some(CartType::ThreeEPlus),
        "DPC" => Some(CartType::Dpc),
        "AR" => Some(CartType::Ar),
        "F0" => Some(CartType::F0),
        "JANE" => Some(CartType::Jane),
        "WF8" => Some(CartType::Wf8),
        "WD" => Some(CartType::Wd),
        "0FA0" => Some(CartType::ZeroFa0),
        "03E0" => Some(CartType::Zero3E0),
        "0840" => Some(CartType::Zero840),
        "EF" => Some(CartType::Ef),
        "DF" => Some(CartType::Df),
        "BF" => Some(CartType::Bf),
        "SB" => Some(CartType::Sb),
        "X07" => Some(CartType::X07),
        "MDM" => Some(CartType::Mdm),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_fields_splits_ntsc_from_pal() {
        assert_eq!(classify_fields(&[42, 262, 262, 262]), TvStandard::Ntsc);
        assert_eq!(classify_fields(&[42, 312, 312, 312]), TvStandard::Pal);
        assert_eq!(classify_fields(&[]), TvStandard::Ntsc);
        // A long startup field doesn't sway the median.
        assert_eq!(
            classify_fields(&[45, 285, 282, 262, 262, 262]),
            TvStandard::Ntsc
        );
    }

    #[test]
    fn probe_reads_ntsc_from_a_real_rom() {
        // A real 8 KB (F8) ROM, size-detected: build, run, and count its fields.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/accuracy/roms/cartridge/bank-f8_ntsc.a26");
        assert_eq!(
            probe_tv_standard(&std::fs::read(&path).unwrap(), None),
            TvStandard::Ntsc
        );
    }
}
