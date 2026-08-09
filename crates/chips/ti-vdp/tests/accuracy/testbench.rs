//! The SG-1000 common-subset testbench: Z80 + 1 KB RAM + 32 KB cartridge +
//! the VDP, interleaved at instruction granularity (the VDP advances by
//! 3 dots per 2 T-states after each instruction). Not a console — the ROMs
//! confine themselves to the envelope every host machine shares.

use missingno_ti_vdp::{Standard, Vdp};
use missingno_zilog_z80::{Bus, Cpu};
use std::path::Path;

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

fn run(rom: &str, budget_frames: u64) -> Verdict {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(rom);
    let cart = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));

    let mut board = Board {
        cart,
        ram: [0; RAM_MASK + 1],
        vdp: Vdp::new(Standard::Ntsc),
    };
    let mut cpu = Cpu::new();

    let budget = budget_frames * TSTATES_PER_FRAME;
    let mut tstates: u64 = 0;
    let mut dots_run: u64 = 0;
    while tstates < budget {
        tstates += cpu.step(&mut board) as u64;
        let dots_due = tstates * 3 / 2;
        board.vdp.tick((dots_due - dots_run) as u32);
        dots_run = dots_due;
        cpu.set_irq(board.vdp.interrupt_asserted());

        let result = board.ram[0];
        if result == RESULT_PASS || result == RESULT_FAIL {
            return Verdict {
                passed: result == RESULT_PASS,
                code: board.ram[1],
                observed: board.ram[2],
                expected: board.ram[3],
            };
        }
    }
    panic!("{rom}: no verdict within {budget_frames} frames");
}

pub fn assert_pass(rom: &str) {
    assert_pass_within(rom, DEFAULT_BUDGET_FRAMES);
}

pub fn assert_pass_within(rom: &str, budget_frames: u64) {
    let verdict = run(rom, budget_frames);
    assert!(
        verdict.passed,
        "{rom}: FAIL code={:02X} observed={:02X} expected={:02X}",
        verdict.code, verdict.observed, verdict.expected
    );
}
