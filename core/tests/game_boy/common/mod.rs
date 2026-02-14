use std::path::{Path, PathBuf};

use missingno_core::game_boy::{GameBoy, cartridge::Cartridge, cpu::Cpu, video::screen::Screen};

fn rom_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/game_boy/roms")
        .join(relative)
}

pub fn load_rom(relative: &str) -> GameBoy {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    GameBoy::new(Cartridge::new(rom, None))
}

/// Run the emulator until the serial output contains any of the given needle strings,
/// or until an infinite loop is detected at a frame boundary, or until a timeout is reached.
pub fn run_until_serial_match(gb: &mut GameBoy, needles: &[&str], timeout_frames: u32) -> String {
    let mut output = String::new();
    for _ in 0..timeout_frames {
        while !gb.step() {}
        let bytes = gb.drain_serial_output();
        if !bytes.is_empty() {
            output.push_str(&String::from_utf8_lossy(&bytes));
            if needles.iter().any(|needle| output.contains(needle)) {
                return output;
            }
        }
        if is_infinite_loop(gb) {
            return output;
        }
    }
    output
}

/// Run the emulator until it enters an infinite loop, or until a timeout (in frames) is reached.
///
/// After the frame-by-frame scan, does one final per-instruction scan (one frame's worth
/// of T-cycles) to catch HALT-based loops that aren't visible at frame boundaries.
pub fn run_until_infinite_loop(gb: &mut GameBoy, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        while !gb.step() {}
        if is_infinite_loop(gb) {
            return true;
        }
    }
    // Per-instruction scan for HALT-based completion loops
    for _ in 0..70224 {
        gb.step();
        if is_infinite_loop(gb) {
            return true;
        }
    }
    false
}

/// Run the emulator until `LD B,B` (opcode 0x40) is about to execute, or until a timeout.
///
/// The Mealybug Tearoom test suite uses `LD B,B` as a software breakpoint to signal
/// "take a screenshot now." The ROM continues running after the breakpoint, so we
/// detect it per-instruction rather than waiting for an infinite loop.
pub fn run_until_breakpoint(gb: &mut GameBoy, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        loop {
            // Check for LD B,B breakpoint before executing
            let pc = gb.cpu().program_counter;
            if gb.memory_mapped().read(pc) == 0x40 {
                return true;
            }
            if gb.step() {
                break;
            }
        }
    }
    false
}

/// Check if the CPU is stuck in a known completion loop.
fn is_infinite_loop(gb: &GameBoy) -> bool {
    let pc = gb.cpu().program_counter;
    let mem = gb.memory_mapped();

    // JR -2 (0x18 0xFE) — standard completion loop
    if mem.read(pc) == 0x18 && mem.read(pc.wrapping_add(1)) == 0xFE {
        return true;
    }

    if gb.cpu().halted {
        // Permanent halt: IE register is empty, so no interrupt can ever wake
        // the CPU. Used by SameSuite's exit sequence (di; IE=0; halt; nop).
        if gb.interrupts().enabled.is_empty() {
            return true;
        }

        // HALT-based loops: when halted, PC is past the HALT instruction.
        // Check if HALT at pc-1 is part of a small backward-jumping loop.
        if mem.read(pc.wrapping_sub(1)) == 0x76 {
            // Scan the few bytes after HALT for a JR that jumps back to or before the HALT
            for offset in 0u16..4 {
                let addr = pc.wrapping_add(offset);
                if mem.read(addr) == 0x18 {
                    let rel = mem.read(addr.wrapping_add(1)) as i8;
                    // JR target = addr + 2 + rel; loop if target <= HALT address (pc-1)
                    let target = addr.wrapping_add(2).wrapping_add(rel as u16);
                    if target <= pc.wrapping_sub(1) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn check_mooneye_pass(cpu: &Cpu) -> bool {
    cpu.b == 3 && cpu.c == 5 && cpu.d == 8 && cpu.e == 13 && cpu.h == 21 && cpu.l == 34
}

pub fn format_registers(cpu: &Cpu) -> String {
    format!(
        "B={} C={} D={} E={} H={} L={} (expected: B=3 C=5 D=8 E=13 H=21 L=34)",
        cpu.b, cpu.c, cpu.d, cpu.e, cpu.h, cpu.l
    )
}

/// Convert a Screen to a flat greyscale pixel buffer using dmg-acid2 reference palette:
/// PaletteIndex 0 → 0xFF, 1 → 0xAA, 2 → 0x55, 3 → 0x00
pub fn screen_to_greyscale(screen: &Screen) -> Vec<u8> {
    const GREYSCALE: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];
    (0..144u8)
        .flat_map(|y| (0..160u8).map(move |x| GREYSCALE[screen.pixel(x, y).0 as usize]))
        .collect()
}

/// Load a reference PNG as a flat greyscale pixel buffer (values 0x00-0xFF).
pub fn load_reference_png(relative: &str) -> Vec<u8> {
    let path = rom_path(relative);
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("Failed to open reference image {}: {e}", path.display()));
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();

    let width = info.width as usize;
    let height = info.height as usize;
    let stride = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("Unsupported PNG color type: {other:?}"),
    };

    (0..width * height).map(|i| buf[i * stride]).collect()
}
