//! The whole console under the corpus: the BIOS from `COLECOVISION_BIOS`, a
//! `.col` image in the slot, and the RESULT block read out of RAM at $7000 at
//! every instruction boundary. Without the variable every subject prints a
//! skip line and returns — the BIOS is never committed.

use missingno_colecovision::console::{ColecoVision, tstates_per_frame};
use missingno_colecovision::firmware::BIOS_SIZE;
use missingno_test_support::compare::{self, assert_pixels_match};
use missingno_test_support::reference::ReferencePng;
use missingno_test_support::verdict::{Outcome, Poll, Verdict, poll_verdict};
use missingno_ti_vdp::{ACTIVE_LINES, ACTIVE_WIDTH, Frame, LEFT_BORDER, PALETTE, Standard};
use std::path::Path;

/// The RESULT block: the base of the conventional $7000 RAM image.
const RESULT_BLOCK: u16 = 0x7000;
/// The VDP's periods per T-state, for advancing the raster alone.
const XTALS_PER_TSTATE: u32 = 3;
/// Generous default: the timing sweeps run ~550 frames to their verdict.
const DEFAULT_BUDGET_FRAMES: u64 = 1200;

/// The chip crate's references: SC-3000 captures of the same scenes. No
/// ColecoVision capture exists.
const REFERENCES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../chips/ti-vdp/tests/accuracy/roms/"
);

/// The BIOS image, or `None` when the variable is unset.
fn bios(rom: &str) -> Option<[u8; BIOS_SIZE]> {
    let Some(path) = std::env::var_os("COLECOVISION_BIOS") else {
        println!("{rom}: skipped — set COLECOVISION_BIOS");
        return None;
    };
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", Path::new(&path).display()));
    Some(
        bytes
            .try_into()
            .unwrap_or_else(|bytes: Vec<u8>| panic!("a {}-byte BIOS image", bytes.len())),
    )
}

fn run(
    rom: &str,
    budget_frames: u64,
    standard: Standard,
    bios: [u8; BIOS_SIZE],
) -> (ColecoVision, Verdict) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(rom);
    let image = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let mut console = ColecoVision::new(&image, standard, bios).expect("a flat cartridge image");

    let budget = budget_frames * u64::from(tstates_per_frame(standard));
    let outcome = poll_verdict(budget, || {
        console.step_tstate();
        if !console.at_instruction_boundary() {
            return Poll::Pending;
        }
        Poll::Read([0, 1, 2, 3].map(|offset| console.peek(RESULT_BLOCK + offset)))
    });

    match outcome {
        Outcome::Reached(verdict) => (console, verdict),
        _ => panic!("{rom}: no verdict within {budget_frames} frames"),
    }
}

/// The corpus's skip: PASS magic with CODE $00 and EXPECTED $FF, the skip's
/// own code in OBSERVED.
fn is_skip(verdict: &Verdict, observed: u8) -> bool {
    verdict.passed
        && verdict.code == 0x00
        && verdict.observed == observed
        && verdict.expected == 0xFF
}
/// "NTSC ONLY": the SG-1000 build of a phase-anchor ROM on 313 lines.
const NTSC_ONLY: u8 = 0xFF;
/// "SG-1000 BUS ONLY": a ROM cycle-counted for the wait-state-free bus.
const BUS_ONLY: u8 = 0xFD;

/// A skip is never a pass: a ROM that declines this machine must say so loudly.
fn refuse_skip(rom: &str, standard: Standard, verdict: &Verdict) {
    for (sentinel, name) in [(NTSC_ONLY, "NTSC ONLY"), (BUS_ONLY, "SG-1000 BUS ONLY")] {
        assert!(
            !is_skip(verdict, sentinel),
            "{rom}: skipped on {standard:?} ({name} sentinel)"
        );
    }
}

pub fn assert_pass(rom: &str, standard: Standard) {
    assert_pass_within(rom, standard, DEFAULT_BUDGET_FRAMES);
}

pub fn assert_pass_within(rom: &str, standard: Standard, budget_frames: u64) {
    let Some(bios) = bios(rom) else {
        return;
    };
    let (_, verdict) = run(rom, budget_frames, standard, bios);
    refuse_skip(rom, standard, &verdict);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X}",
        verdict.code, verdict.observed, verdict.expected
    );
}

/// A ROM cycle-counted for the SG-1000's bus: it must latch the BUS ONLY
/// sentinel rather than run.
pub fn assert_bus_only(rom: &str, standard: Standard) {
    let Some(bios) = bios(rom) else {
        return;
    };
    let (_, verdict) = run(rom, DEFAULT_BUDGET_FRAMES, standard, bios);
    assert!(
        is_skip(&verdict, BUS_ONLY),
        "{rom}: expected the SG-1000 BUS ONLY sentinel on {standard:?}, got {} code={:02X} observed={:02X} expected={:02X}",
        if verdict.passed { "PASS" } else { "FAIL" },
        verdict.code,
        verdict.observed,
        verdict.expected
    );
}

const MAX_REPORTED_MISMATCHES: usize = 16;

/// The display area of a visible raster, row-major.
fn active_area(frame: &Frame, standard: Standard) -> Vec<u8> {
    let width = frame.width as usize;
    let top = standard.top_border() as usize;
    (0..ACTIVE_LINES as usize)
        .flat_map(|y| {
            let start = (top + y) * width + LEFT_BORDER as usize;
            frame.pixels[start..start + ACTIVE_WIDTH as usize].to_vec()
        })
        .collect()
}

/// Run a screenshot subject to its PASS verdict, advance the raster to the
/// next complete frame, and diff its display area against the chip crate's
/// 256x192 reference pixel-exactly.
pub fn assert_screenshot(rom: &str, standard: Standard) {
    let Some(bios) = bios(rom) else {
        return;
    };
    let (mut console, verdict) = run(rom, DEFAULT_BUDGET_FRAMES, standard, bios);
    refuse_skip(rom, standard, &verdict);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X} before its scene settled",
        verdict.code, verdict.observed, verdict.expected
    );

    // The scene is latched; only the raster needs to advance to the next
    // complete frame.
    let captured_at = console.vdp().frames_completed() + 1;
    while console.vdp().frames_completed() < captured_at {
        console.vdp_mut().tick(XTALS_PER_TSTATE);
    }
    let active = active_area(console.vdp().frame(), standard);

    if let Ok(dir) = std::env::var("COLECOVISION_DUMP_FRAMES") {
        dump_frame(&dir, rom, standard, &active);
    }

    let Some(reference) = load_reference(rom) else {
        panic!("{rom}: no blessed reference committed");
    };
    let actual: Vec<[u8; 3]> = active
        .iter()
        .map(|&index| PALETTE[index as usize])
        .collect();
    assert_pixels_match(
        rom,
        &actual,
        &reference,
        256,
        MAX_REPORTED_MISMATCHES,
        compare::debug_value,
    );
}

/// `COLECOVISION_DUMP_FRAMES=<dir>` writes each captured display area through
/// the chip's canonical palette.
fn dump_frame(dir: &str, rom: &str, standard: Standard, active: &[u8]) {
    let stem = Path::new(rom).file_stem().unwrap().to_string_lossy();
    let body = match standard {
        Standard::Ntsc => "",
        Standard::Pal => "_pal",
    };
    let path = Path::new(dir).join(format!("{stem}_missingno{body}.png"));
    std::fs::create_dir_all(dir).unwrap();
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 256, 192);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let mut data = Vec::with_capacity(256 * 192 * 3);
    for &index in active {
        data.extend_from_slice(&PALETTE[index as usize]);
    }
    writer.write_image_data(&data).unwrap();
}

fn load_reference(rom: &str) -> Option<Vec<[u8; 3]>> {
    let stem = rom.strip_suffix(".col").unwrap_or(rom);
    let path = Path::new(REFERENCES).join(format!("{stem}_ntsc.png"));
    if !path.exists() {
        return None;
    }
    let reference = ReferencePng::load(&path);
    reference.require_colour();
    assert_eq!(
        (reference.width(), reference.height()),
        (256, 192),
        "{}: reference must be the 256x192 active area",
        path.display()
    );
    Some(reference.rgb())
}
