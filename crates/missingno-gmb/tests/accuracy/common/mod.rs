use std::path::{Path, PathBuf};

use missingno_gmb::{GameBoy, cartridge::Cartridge, cpu::Cpu, execute::StepResult, ppu::screen::Screen};

#[cfg(feature = "gbtrace")]
use missingno_gmb::trace::Tracer;

fn rom_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(relative)
}

/// A test run wrapping a GameBoy and an optional trace writer.
///
/// When the `gbtrace` feature is enabled and the `GBTRACE_PROFILE` env var
/// is set (to a profile name like `cpu_basic`), each `step()` captures state
/// into a parquet trace file under `receipts/traces/`.
pub struct TestRun {
    pub gb: GameBoy,
    #[cfg(feature = "gbtrace")]
    tracer: Option<Tracer>,
}

impl TestRun {
    fn new(gb: GameBoy, _rom_relative: &str) -> Self {
        #[cfg(feature = "gbtrace")]
        let tracer = try_create_tracer(&gb, _rom_relative);

        Self {
            gb,
            #[cfg(feature = "gbtrace")]
            tracer,
        }
    }

    /// Step one instruction, capturing trace state if active.
    ///
    /// For tcycle-triggered profiles, this steps dot-by-dot and captures
    /// state at every T-cycle. For instruction-triggered profiles, it
    /// captures once before the instruction executes.
    pub fn step(&mut self) -> StepResult {
        #[cfg(feature = "gbtrace")]
        {
            if let Some(tracer) = &mut self.tracer {
                if tracer.trigger() == missingno_gmb::trace::Trigger::Tcycle {
                    return self.step_traced_tcycle();
                }
                tracer.capture(&self.gb).unwrap();
            }

            let result = self.gb.step();

            if let Some(tracer) = &mut self.tracer {
                tracer.advance(result.dots);
                if result.new_screen {
                    tracer.mark_frame().unwrap();
                }
            }

            return result;
        }

        #[cfg(not(feature = "gbtrace"))]
        self.gb.step()
    }

    /// Step one instruction by advancing one dot at a time, capturing at each dot.
    ///
    /// Uses `step_phase()` to advance half-dots, capturing state at each
    /// dot boundary (when clock returns to Low). Detects instruction
    /// boundaries via the CPU's boundary flag to know when the instruction
    /// is complete.
    #[cfg(feature = "gbtrace")]
    fn step_traced_tcycle(&mut self) -> StepResult {
        let mut new_screen = false;
        let mut dots = 0u32;

        // Consume the current instruction boundary so we can detect the next one.
        self.gb.cpu_mut().take_instruction_boundary();

        loop {
            // Execute rise phase — feed any pixel to the tracer.
            let rise = self.gb.step_phase();
            new_screen |= rise.new_screen;
            if let Some(pixel) = rise.pixel {
                self.tracer.as_mut().unwrap().push_pixel(pixel.shade);
            }

            // Execute fall phase.
            let fall = self.gb.step_phase();
            new_screen |= fall.new_screen;
            if let Some(pixel) = fall.pixel {
                self.tracer.as_mut().unwrap().push_pixel(pixel.shade);
            }

            // Capture state after both phases — pix buffer contains this
            // dot's pixel output, registers reflect post-phase state.
            // Matches GateBoy's convention of capturing at end of tcycle.
            let tracer = self.tracer.as_mut().unwrap();
            tracer.capture(&self.gb).unwrap();
            tracer.advance_dot();
            dots += 1;

            if self.gb.cpu().at_instruction_boundary() {
                break;
            }
        }

        StepResult { new_screen, dots }
    }

    /// Finalize the trace file (if active). Call when the test is done.
    pub fn finish(mut self) {
        #[cfg(feature = "gbtrace")]
        if let Some(tracer) = self.tracer.take() {
            tracer.finish().unwrap();
        }
    }
}

#[cfg(feature = "gbtrace")]
impl Drop for TestRun {
    fn drop(&mut self) {
        if let Some(tracer) = self.tracer.take() {
            let _ = tracer.finish();
        }
    }
}

#[cfg(feature = "gbtrace")]
fn try_create_tracer(gb: &GameBoy, rom_relative: &str) -> Option<Tracer> {
    let profile_name = std::env::var("GBTRACE_PROFILE").ok()?;

    let gbtrace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../gbtrace");
    // Search for the profile in several locations:
    // 1. profiles/<name>.toml (standard profiles)
    // 2. test-suites/<name>/profile.toml (test-suite-specific)
    // 3. docs/tests/<name>/<name>.toml (legacy location)
    let profile_path = {
        let candidates = [
            gbtrace_root.join("profiles").join(format!("{profile_name}.toml")),
            gbtrace_root.join("test-suites").join(&profile_name).join("profile.toml"),
            gbtrace_root.join("docs/tests").join(&profile_name).join(format!("{profile_name}.toml")),
        ];
        candidates.into_iter().find(|p| p.exists())
            .unwrap_or_else(|| panic!("gbtrace profile '{profile_name}' not found in any search path"))
    };
    let profile = gbtrace::Profile::load(&profile_path)
        .unwrap_or_else(|e| panic!("Failed to load gbtrace profile {}: {e}", profile_path.display()));

    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../receipts/traces");
    std::fs::create_dir_all(&output_dir).unwrap();

    // Use the ROM filename (without extension) as the trace filename.
    let rom_stem = Path::new(rom_relative)
        .file_stem()
        .unwrap()
        .to_string_lossy();
    let output_path = output_dir.join(format!("{rom_stem}.parquet"));

    let boot_rom = if gb.cpu().program_counter == 0x0000 {
        gbtrace::BootRom::Skip // TODO: detect actual boot ROM
    } else {
        gbtrace::BootRom::Skip
    };

    eprintln!("gbtrace: writing {}", output_path.display());

    Some(
        Tracer::create(&output_path, &profile, gb, boot_rom)
            .unwrap_or_else(|e| panic!("Failed to create tracer: {e}")),
    )
}

pub fn load_rom(relative: &str) -> TestRun {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    let boot_rom = try_load_boot_rom();
    let mut gb = GameBoy::new(Cartridge::new(rom, None), boot_rom);
    run_boot_rom(&mut gb);
    TestRun::new(gb, relative)
}

/// Load a ROM with a boot ROM. The boot ROM runs from 0x0000 before
/// handing control to the cartridge at 0x0100.
pub fn load_rom_with_boot_rom(relative: &str, boot_rom: Box<[u8; 256]>) -> TestRun {
    let gb = GameBoy::new(Cartridge::new(std::fs::read(rom_path(relative)).unwrap(), None), Some(boot_rom));
    TestRun::new(gb, relative)
}

/// Try to load the DMG boot ROM from the path in `DMG_BOOT_ROM`.
/// Returns None if the env var is unset or the file can't be read.
/// The boot ROM cannot be distributed with the repo for legal reasons.
pub fn try_load_boot_rom() -> Option<Box<[u8; 256]>> {
    let path = std::env::var("DMG_BOOT_ROM").ok()?;
    let data = std::fs::read(&path).ok()?;
    let boxed: Box<[u8; 256]> = data.into_boxed_slice().try_into().ok()?;
    Some(boxed)
}

/// If a boot ROM is loaded, run it to completion (PC reaches 0x0100).
/// This is a no-op when no boot ROM is present.
fn run_boot_rom(gb: &mut GameBoy) {
    if gb.cpu().program_counter != 0x0000 {
        return;
    }
    for _ in 0..10_000_000 {
        gb.step();
        if gb.cpu().program_counter == 0x0100 {
            return;
        }
    }
    panic!(
        "Boot ROM did not reach 0x0100 within 10M steps — does the ROM have a valid Nintendo logo?"
    );
}

/// Run the emulator until the serial output contains any of the given needle strings,
/// or until an infinite loop is detected at a frame boundary, or until a timeout is reached.
pub fn run_until_serial_match(run: &mut TestRun, needles: &[&str], timeout_frames: u32) -> String {
    let mut output = String::new();
    for _ in 0..timeout_frames {
        while !run.step().new_screen {}
        let bytes = run.gb.drain_serial_output();
        if !bytes.is_empty() {
            output.push_str(&String::from_utf8_lossy(&bytes));
            if needles.iter().any(|needle| output.contains(needle)) {
                return output;
            }
        }
        if is_infinite_loop(&run.gb) {
            return output;
        }
    }
    output
}

/// Run the emulator for a fixed number of frames. Used for ROMs that display
/// results but don't terminate with an infinite loop.
pub fn run_frames(run: &mut TestRun, frames: u32) {
    for _ in 0..frames {
        while !run.step().new_screen {}
    }
}

/// Run the emulator until it enters an infinite loop, or until a timeout (in frames) is reached.
///
/// After the frame-by-frame scan, does one final per-instruction scan (one frame's worth
/// of T-cycles) to catch HALT-based loops that aren't visible at frame boundaries.
pub fn run_until_infinite_loop(run: &mut TestRun, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        while !run.step().new_screen {}
        if is_infinite_loop(&run.gb) {
            return true;
        }
    }
    // Per-instruction scan for HALT-based completion loops
    for _ in 0..70224 {
        run.step();
        if is_infinite_loop(&run.gb) {
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
pub fn run_until_breakpoint(run: &mut TestRun, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        loop {
            // Check for LD B,B breakpoint before executing
            let pc = run.gb.cpu().program_counter;
            if run.gb.read(pc) == 0x40 {
                return true;
            }
            if run.step().new_screen {
                break;
            }
        }
    }
    false
}

/// Run the emulator until opcode 0xED (undefined) is about to execute, or until
/// an infinite loop is detected, or until a timeout. The wilbertpol Mooneye fork
/// uses 0xED as its test exit condition.
pub fn run_until_undefined_opcode(run: &mut TestRun, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        loop {
            let pc = run.gb.cpu().program_counter;
            if run.gb.read(pc) == 0xED {
                return true;
            }
            if is_infinite_loop(&run.gb) {
                return true;
            }
            if run.step().new_screen {
                break;
            }
        }
    }
    false
}

/// Check if the CPU is stuck in a known completion loop.
fn is_infinite_loop(gb: &GameBoy) -> bool {
    let pc = gb.cpu().program_counter;
    // JR -2 (0x18 0xFE) — standard completion loop
    if gb.read(pc) == 0x18 && gb.read(pc.wrapping_add(1)) == 0xFE {
        return true;
    }

    if gb.cpu().halt_state != missingno_gmb::cpu::HaltState::Running {
        // Permanent halt: IE register is empty, so no interrupt can ever wake
        // the CPU. Used by SameSuite's exit sequence (di; IE=0; halt; nop).
        if gb.interrupts().enabled.is_empty() {
            return true;
        }

        // HALT-based loops: when halted, PC is past the HALT instruction.
        // Check if HALT at pc-1 is part of a small backward-jumping loop.
        if gb.read(pc.wrapping_sub(1)) == 0x76 {
            // Scan the few bytes after HALT for a JR that jumps back to or before the HALT
            for offset in 0u16..4 {
                let addr = pc.wrapping_add(offset);
                if gb.read(addr) == 0x18 {
                    let rel = gb.read(addr.wrapping_add(1)) as i8;
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
