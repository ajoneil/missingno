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

use missingno_vcs::console::{Frame, Vcs};
use missingno_vcs::tia::{VISIBLE_CLOCKS, palette, palette_index};
use missingno_vcs::{CartType, DumpFit, TvStandard};

/// RESULT convention (see `missingno-vcs-tests/include/result.h`).
const RESULT: u16 = 0x0080;
const CODE: u16 = 0x0081;
const OBSERVED: u16 = 0x0082;
const EXPECTED: u16 = 0x0083;
const PASS_MAGIC: u8 = 0xA5;
const FAIL_MAGIC: u8 = 0x5A;

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
fn load(relative: &str, standard: TvStandard, cart_type: CartType) -> Vcs {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read ROM {}: {e}", path.display()));
    Vcs::new(&rom, standard, Some(cart_type), DumpFit::Exact)
        .unwrap_or_else(|e| panic!("failed to load ROM {}: {e:?}", path.display()))
}

/// Run a self-checking image the test built itself, for a board whose subject
/// is the container rather than any one program.
pub fn run_self_test_image(name: &str, image: &[u8], standard: TvStandard, cart_type: CartType) {
    let vcs = Vcs::new(image, standard, Some(cart_type), DumpFit::Exact)
        .unwrap_or_else(|e| panic!("failed to load {name}: {e:?}"));
    poll_verdict(name, vcs);
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
    poll_verdict(relative, load(relative, standard, cart_type));
}

fn poll_verdict(relative: &str, mut vcs: Vcs) {
    for _ in 0..MAX_INSTRUCTIONS {
        vcs.step_instruction();
        match vcs.peek(RESULT) {
            PASS_MAGIC => return,
            FAIL_MAGIC => panic!(
                "{relative}: FAIL code=0x{:02X} observed=0x{:02X} expected=0x{:02X}",
                vcs.peek(CODE),
                vcs.peek(OBSERVED),
                vcs.peek(EXPECTED),
            ),
            _ => {}
        }
        if vcs.cpu.jammed() {
            break;
        }
    }

    if vcs.cpu.jammed() {
        panic!(
            "{relative}: CPU halted (JAM) before any verdict (RESULT=0x{:02X})",
            vcs.peek(RESULT)
        );
    }
    panic!(
        "{relative}: no verdict (RESULT=0x{:02X}) within instruction budget",
        vcs.peek(RESULT)
    );
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

    let reference = Png::load(&rom_path(png_relative));
    assert_eq!(
        reference.width, VISIBLE_CLOCKS,
        "{png_relative}: reference width must be the TIA's visible clocks"
    );

    // References carry the VSYNC-period lines that `Frame` drops, so they
    // run a few lines taller; compare the overlap.
    let rows = frame.lines.len().min(reference.height);
    let mut mismatches = 0;
    for y in 0..rows {
        for x in 0..VISIBLE_CLOCKS {
            let a = actual[y * VISIBLE_CLOCKS + x];
            let e = reference.rgb(x, y);
            if a != e {
                if mismatches < MAX_REPORTED_MISMATCHES {
                    eprintln!("{rom_relative}: pixel ({x},{y}) got {a:?} expected {e:?}");
                }
                mismatches += 1;
            }
        }
    }

    assert_eq!(
        mismatches, 0,
        "{rom_relative}: {mismatches} pixel mismatches vs {png_relative}"
    );
}

fn frame_to_rgb(frame: &Frame, standard: TvStandard) -> Vec<(u8, u8, u8)> {
    let palette = palette(standard);
    frame
        .lines
        .iter()
        .flat_map(|line| line.iter().map(|&byte| palette[palette_index(byte)]))
        .collect()
}

/// A decoded reference frame, RGB per pixel.
struct Png {
    width: usize,
    height: usize,
    rgb: Vec<(u8, u8, u8)>,
}

impl Png {
    fn load(path: &Path) -> Png {
        let file = std::fs::File::open(path)
            .unwrap_or_else(|e| panic!("failed to open reference {}: {e}", path.display()));
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();

        let stride = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => panic!("unsupported reference PNG colour type: {other:?}"),
        };
        let rgb = (0..info.width as usize * info.height as usize)
            .map(|i| (buf[i * stride], buf[i * stride + 1], buf[i * stride + 2]))
            .collect();

        Png {
            width: info.width as usize,
            height: info.height as usize,
            rgb,
        }
    }

    fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        self.rgb[y * self.width + x]
    }
}
