//! The SG-1000 common-subset testbench: Z80 + 1 KB RAM + 32 KB cartridge +
//! the VDP, interleaved one Z80 T-state at a time — the VDP advances by that
//! T's three crystal periods before the CPU ticks, so a port access lands
//! against a VDP that has already reached the instant it fires on. Not a
//! console — the ROMs confine themselves to the envelope every host machine
//! shares.

use missingno_ti_vdp::{Standard, Vdp, XTALS_PER_TSTATE};
use missingno_zilog_z80::{Bus, Cpu};
use std::path::Path;

#[cfg(feature = "morepork")]
mod trace;

const RAM_BASE: u16 = 0xC000;
const RAM_MASK: usize = 0x3FF;

const RESULT_PASS: u8 = 0xA5;
const RESULT_FAIL: u8 = 0x5A;

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

pub struct Verdict {
    pub passed: bool,
    pub code: u8,
    pub observed: u8,
    pub expected: u8,
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

    let budget = budget_frames * TSTATES_PER_FRAME;
    for _ in 0..budget {
        board.vdp.tick(XTALS_PER_TSTATE);
        cpu.tick(&mut board);
        cpu.set_irq(board.vdp.interrupt_asserted());

        if !cpu.at_instruction_boundary() {
            continue;
        }
        #[cfg(feature = "morepork")]
        if let Some(tracer) = &mut tracer {
            tracer.capture(&cpu, &mut board);
        }
        let result = board.ram[0];
        if result == RESULT_PASS || result == RESULT_FAIL {
            #[cfg(feature = "morepork")]
            if let Some(tracer) = tracer.take() {
                tracer.finish(&board.ram);
            }
            let verdict = Verdict {
                passed: result == RESULT_PASS,
                code: board.ram[1],
                observed: board.ram[2],
                expected: board.ram[3],
            };
            return (board, verdict);
        }
    }
    #[cfg(feature = "morepork")]
    if let Some(tracer) = tracer.take() {
        tracer.finish(&board.ram);
    }
    panic!("{rom}: no verdict within {budget_frames} frames");
}

pub fn assert_pass(rom: &str) {
    assert_pass_within(rom, DEFAULT_BUDGET_FRAMES);
}

/// The canonical datasheet palette every capture pipeline stamps (TI defines
/// colours as analog levels; RGB is presentation policy, tests-side only).
/// Index 0 is the all-planes-transparent pass-through and presents as black,
/// so RGB equality is colour-index equality for every rendered pixel.
const TI_PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 0],
    [33, 200, 66],
    [94, 220, 120],
    [84, 85, 237],
    [125, 118, 252],
    [212, 82, 77],
    [66, 235, 245],
    [252, 85, 84],
    [255, 121, 120],
    [212, 193, 84],
    [230, 206, 128],
    [33, 176, 59],
    [201, 91, 186],
    [204, 204, 204],
    [255, 255, 255],
];

const MAX_REPORTED_MISMATCHES: usize = 16;

/// Run a screenshot subject to its PASS verdict (the scene is up and stays
/// up once the verdict latches), capture the next complete frame, and diff
/// it against the blessed 256x192 reference pixel-exactly.
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
    let frame = board.vdp.frame();

    if let Ok(dir) = std::env::var("TIVDP_DUMP_FRAMES") {
        dump_frame(&dir, rom, frame);
    }

    let reference = load_reference(rom);
    let mut mismatches = 0usize;
    for (y, row) in frame.0.iter().enumerate() {
        for (x, &index) in row.iter().enumerate() {
            let actual = TI_PALETTE[index as usize];
            let expected = reference[y * 256 + x];
            if actual != expected {
                if mismatches < MAX_REPORTED_MISMATCHES {
                    eprintln!("{rom}: pixel ({x},{y}) got {actual:?} expected {expected:?}");
                }
                mismatches += 1;
            }
        }
    }
    assert_eq!(mismatches, 0, "{rom}: {mismatches} pixel mismatches");
}

/// `TIVDP_DUMP_FRAMES=<dir>` writes each captured frame through the
/// canonical palette — the working tool for diffing a divergent scene.
fn dump_frame(dir: &str, rom: &str, frame: &missingno_ti_vdp::Frame) {
    let stem = Path::new(rom).file_stem().unwrap().to_string_lossy();
    let path = Path::new(dir).join(format!("{stem}_missingno.png"));
    std::fs::create_dir_all(dir).unwrap();
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 256, 192);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let mut data = Vec::with_capacity(256 * 192 * 3);
    for row in &frame.0 {
        for &index in row {
            data.extend_from_slice(&TI_PALETTE[index as usize]);
        }
    }
    writer.write_image_data(&data).unwrap();
}

fn load_reference(rom: &str) -> Vec<[u8; 3]> {
    let stem = rom.strip_suffix(".sg").unwrap_or(rom);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(format!("{stem}_ntsc.png"));
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("opening reference {}: {e}", path.display()));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(
        (info.width, info.height),
        (256, 192),
        "{}: reference must be the 256x192 active area",
        path.display()
    );
    let stride = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unsupported reference colour type: {other:?}"),
    };
    (0..256 * 192)
        .map(|i| [buf[i * stride], buf[i * stride + 1], buf[i * stride + 2]])
        .collect()
}

pub fn assert_pass_within(rom: &str, budget_frames: u64) {
    let (_, verdict) = run(rom, budget_frames);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X}",
        verdict.code, verdict.observed, verdict.expected
    );
}
