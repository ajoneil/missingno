//! The SG-1000 common-subset testbench: Z80 + 1 KB RAM + 32 KB cartridge +
//! the VDP, interleaved one Z80 T-state at a time — the VDP advances by that
//! T's three crystal periods before the CPU ticks, so a port access lands
//! against a VDP that has already reached the instant it fires on. Not a
//! console — the ROMs confine themselves to the envelope every host machine
//! shares.

use missingno_test_support::compare::{self, assert_pixels_match};
use missingno_test_support::reference::ReferencePng;
use missingno_test_support::verdict::{Outcome, Poll, Verdict, poll_verdict};
use missingno_ti_vdp::{ACTIVE_LINES, ACTIVE_WIDTH, Frame, LEFT_BORDER, PALETTE, Standard, Vdp};
use missingno_zilog_z80::{Bus, Cpu};
use std::path::Path;

#[cfg(feature = "morepork")]
mod trace;

const RAM_BASE: u16 = 0xC000;
const RAM_MASK: usize = 0x3FF;

/// The envelope's crystal against its Z80: 10.738635 MHz over 3.579545 MHz.
const XTALS_PER_TSTATE: u32 = 3;
const TSTATES_PER_FRAME: u64 = 228 * 262;
/// Generous default: the timing sweeps run ~550 frames to their verdict.
const DEFAULT_BUDGET_FRAMES: u64 = 1200;

pub struct Board {
    cart: Vec<u8>,
    ram: [u8; RAM_MASK + 1],
    pub vdp: Vdp,
    #[cfg(feature = "morepork")]
    ram_write: Option<(u16, u8)>,
}

impl Bus for Board {
    fn read(&mut self, address: u16) -> u8 {
        if address < 0x8000 {
            self.cart.get(address as usize).copied().unwrap_or(0xFF)
        } else if address >= RAM_BASE {
            self.ram[address as usize & RAM_MASK]
        } else {
            0xFF
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        if address >= RAM_BASE {
            self.ram[address as usize & RAM_MASK] = data;
            #[cfg(feature = "morepork")]
            {
                self.ram_write = Some((address, data));
            }
        }
    }

    // The SG-1000 decodes the VDP across the whole $80-$BF window, A0
    // selecting data (even) or control/status (odd).
    fn input(&mut self, port: u16) -> u8 {
        match port as u8 {
            0x80..=0xBF => {
                if port & 1 == 0 {
                    self.vdp.read_data()
                } else {
                    self.vdp.read_status()
                }
            }
            _ => 0xFF,
        }
    }

    fn output(&mut self, port: u16, data: u8) {
        if let 0x80..=0xBF = port as u8 {
            if port & 1 == 0 {
                self.vdp.write_data(data)
            } else {
                self.vdp.write_control(data)
            }
        }
    }
}

fn run(rom: &str, budget_frames: u64) -> (Board, Verdict) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(rom);
    let cart = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));

    let mut board = Board {
        cart,
        ram: [0; RAM_MASK + 1],
        vdp: Vdp::new(Standard::Ntsc),
        #[cfg(feature = "morepork")]
        ram_write: None,
    };
    let mut cpu = Cpu::new();
    #[cfg(feature = "morepork")]
    let mut tracer = trace::Tracer::create(rom, &cpu, &board);

    let outcome = poll_verdict(budget_frames * TSTATES_PER_FRAME, || {
        board.vdp.tick(XTALS_PER_TSTATE);
        cpu.tick(&mut board);
        cpu.set_irq(board.vdp.interrupt_asserted());

        if !cpu.at_instruction_boundary() {
            return Poll::Pending;
        }
        #[cfg(feature = "morepork")]
        if let Some(tracer) = &mut tracer {
            tracer.capture(&cpu, &mut board);
        }
        Poll::Read([board.ram[0], board.ram[1], board.ram[2], board.ram[3]])
    });

    #[cfg(feature = "morepork")]
    if let Some(tracer) = tracer.take() {
        tracer.finish(&board.ram);
    }

    match outcome {
        Outcome::Reached(verdict) => (board, verdict),
        _ => panic!("{rom}: no verdict within {budget_frames} frames"),
    }
}

pub fn assert_pass(rom: &str) {
    assert_pass_within(rom, DEFAULT_BUDGET_FRAMES);
}

const MAX_REPORTED_MISMATCHES: usize = 16;

/// The display area of a visible raster, row-major — the crop the blessed
/// references and the dumps for capture adjudication are both stated in.
fn active_area(frame: &Frame) -> Vec<u8> {
    let width = frame.width as usize;
    let top = Standard::Ntsc.top_border() as usize;
    (0..ACTIVE_LINES as usize)
        .flat_map(|y| {
            let start = (top + y) * width + LEFT_BORDER as usize;
            frame.pixels[start..start + ACTIVE_WIDTH as usize].to_vec()
        })
        .collect()
}

/// Run a screenshot subject to its PASS verdict (the scene is up and stays
/// up once the verdict latches), capture the next complete frame, and diff
/// its display area against the blessed 256x192 reference pixel-exactly.
pub fn assert_screenshot(rom: &str) {
    let (mut board, verdict) = run(rom, DEFAULT_BUDGET_FRAMES);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X} before its scene settled",
        verdict.code, verdict.observed, verdict.expected
    );

    // The scene is latched; only the raster needs to advance to the next
    // complete frame.
    let captured_at = board.vdp.frames_completed() + 1;
    while board.vdp.frames_completed() < captured_at {
        board.vdp.tick(XTALS_PER_TSTATE);
    }
    let active = active_area(board.vdp.frame());

    if let Ok(dir) = std::env::var("TIVDP_DUMP_FRAMES") {
        dump_frame(&dir, rom, &active);
    }

    let Some(reference) = load_reference(rom) else {
        // Hardware-PRIMARY subjects run staged until a capture-derived
        // reference lands; running one explicitly still dumps its frame
        // above for adjudication against the SC-3000 capture.
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

/// `TIVDP_DUMP_FRAMES=<dir>` writes each captured display area through the
/// chip's canonical palette — the working tool for diffing a divergent scene.
fn dump_frame(dir: &str, rom: &str, active: &[u8]) {
    let stem = Path::new(rom).file_stem().unwrap().to_string_lossy();
    let path = Path::new(dir).join(format!("{stem}_missingno.png"));
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
    let stem = rom.strip_suffix(".sg").unwrap_or(rom);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(format!("{stem}_ntsc.png"));
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

pub fn assert_pass_within(rom: &str, budget_frames: u64) {
    let (_, verdict) = run(rom, budget_frames);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X}",
        verdict.code, verdict.observed, verdict.expected
    );
}
