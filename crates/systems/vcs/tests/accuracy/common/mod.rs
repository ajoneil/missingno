//! Shared helpers for the VCS accuracy suite.
//!
//! Two flavours of test, mirroring `missingno-vcs-tests`:
//!
//! * **Self-tests** — the ROM computes its own verdict and writes it to the
//!   RESULT convention in RIOT RAM (`$80` = `$A5` PASS / `$5A` FAIL, with
//!   `$81`/`$82`/`$83` carrying a failing sub-check code and the
//!   observed/expected bytes). [`run_self_test`] polls that block.
//! * **Screenshot tests** — a `_<region>.png` reference sits beside the ROM;
//!   [`run_screenshot`] renders a frame through the TIA palette and diffs it.
//!
//! ROMs and references live under `tests/accuracy/roms/`, filed by subsystem.

use std::path::{Path, PathBuf};

use missingno_core::system::SystemConsole;
use missingno_test_support::compare::{self, assert_pixels_match};
use missingno_test_support::reference::ReferencePng;
use missingno_test_support::verdict::{Outcome, Poll, poll_verdict};
use missingno_vcs::console::{Frame, Vcs};
use missingno_vcs::debug::create_console;
use missingno_vcs::tia::{VISIBLE_CLOCKS, palette, palette_index};
use missingno_vcs::{CartType, DumpFit, TvStandard};

/// RESULT convention (see `missingno-vcs-tests/include/result.h`).
const RESULT: u16 = 0x0080;

/// A self-test writes its verdict within a handful of frames; this caps a
/// hung or never-verdicting ROM. Each instruction is a few CPU cycles — at
/// least six colour clocks — so this is far more headroom than any test needs.
const MAX_INSTRUCTIONS: u64 = 5_000_000;

/// Enough lines to bound a single frame for either region (PAL is 312).
const FRAME_LINE_BUDGET: usize = 400;

/// Cap on the per-pixel mismatch lines printed before the count summary.
const MAX_REPORTED_MISMATCHES: usize = 16;

pub fn rom_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(relative)
}

/// The board a test ROM is loaded on is always stated, never inferred from the
/// image: a size heuristic would silently decide the very thing the cartridge
/// tests exist to check. This mirrors the suite's own `; mapper:` convention,
/// where an unmarked source is a plain 4K board.
pub fn load(relative: &str, standard: TvStandard, cart_type: CartType) -> Vcs {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read ROM {}: {e}", path.display()));
    Vcs::new(&rom, standard, Some(cart_type), DumpFit::Exact)
        .unwrap_or_else(|e| panic!("failed to load ROM {}: {e:?}", path.display()))
}

/// A ROM on the app seam, on the NTSC standard the seam-level tests assume.
pub fn seam_console(relative: &str) -> Box<dyn SystemConsole> {
    let rom = std::fs::read(rom_path(relative)).unwrap();
    create_console(&rom, "test".into(), Some(TvStandard::Ntsc), None, false).unwrap()
}

/// Run a self-checking image the test built itself, for a board whose subject
/// is the container rather than any one program.
pub fn run_self_test_image(name: &str, image: &[u8], standard: TvStandard, cart_type: CartType) {
    let vcs = Vcs::new(image, standard, Some(cart_type), DumpFit::Exact)
        .unwrap_or_else(|e| panic!("failed to load {name}: {e:?}"));
    run_to_verdict(name, vcs);
}

/// Run a self-checking ROM on a plain 4K board — the suite's unmarked default,
/// for tests whose subject is not the cartridge.
pub fn run_self_test(relative: &str, standard: TvStandard) {
    run_self_test_on(relative, standard, CartType::Plain4K);
}

/// Run a self-checking ROM to its verdict on a stated board and assert PASS.
/// Panics with the failing sub-check code and observed/expected bytes on FAIL,
/// or after the instruction budget if the ROM never reports a verdict.
pub fn run_self_test_on(relative: &str, standard: TvStandard, cart_type: CartType) {
    run_to_verdict(relative, load(relative, standard, cart_type));
}

fn run_to_verdict(relative: &str, mut vcs: Vcs) {
    let outcome = poll_verdict(MAX_INSTRUCTIONS, || {
        vcs.step_instruction();
        let block = [0, 1, 2, 3].map(|offset| vcs.peek(RESULT + offset));
        if vcs.cpu.jammed() {
            Poll::Stopped(block)
        } else {
            Poll::Read(block)
        }
    });

    match outcome {
        Outcome::Reached(verdict) if verdict.passed => {}
        Outcome::Reached(verdict) => panic!(
            "{relative}: FAIL code=0x{:02X} observed=0x{:02X} expected=0x{:02X}",
            verdict.code, verdict.observed, verdict.expected
        ),
        Outcome::Stopped(result) => {
            panic!("{relative}: CPU halted (JAM) before any verdict (RESULT=0x{result:02X})")
        }
        Outcome::Exhausted(result) => {
            panic!("{relative}: no verdict (RESULT=0x{result:02X}) within instruction budget")
        }
    }
}

/// Render one settled frame and diff it against the reference PNG, asserting
/// no pixel mismatches. The frame is rendered through the standard's palette,
/// so PAL ROMs decode through the PAL colour table.
pub fn run_screenshot(rom_relative: &str, png_relative: &str, standard: TvStandard) {
    let mut vcs = load(rom_relative, standard, CartType::Plain4K);

    // Let the picture settle; keep the last frame produced within the budget.
    let mut frame = None;
    for _ in 0..8 {
        if let Some(f) = vcs.step_frame(FRAME_LINE_BUDGET) {
            frame = Some(f);
        }
    }
    let frame = frame.unwrap_or_else(|| panic!("{rom_relative}: produced no frame"));
    let actual = frame_to_rgb(&frame, standard);

    let reference = ReferencePng::load(&rom_path(png_relative));
    reference.require_colour();
    assert_eq!(
        reference.width(),
        VISIBLE_CLOCKS,
        "{png_relative}: reference width must be the TIA's visible clocks"
    );

    // References carry the VSYNC-period lines that `Frame` drops, so they
    // run a few lines taller; compare the overlap.
    let rows = frame.lines.len().min(reference.height());
    let pixels = rows * VISIBLE_CLOCKS;
    assert_pixels_match(
        &format!("{rom_relative} vs {png_relative}"),
        &actual[..pixels],
        &reference.rgb()[..pixels],
        VISIBLE_CLOCKS,
        MAX_REPORTED_MISMATCHES,
        compare::debug_value,
    );
}

fn frame_to_rgb(frame: &Frame, standard: TvStandard) -> Vec<[u8; 3]> {
    let palette = palette(standard);
    frame
        .lines
        .iter()
        .flat_map(|line| {
            line.iter().map(|&byte| {
                let (r, g, b) = palette[palette_index(byte)];
                [r, g, b]
            })
        })
        .collect()
}
