use std::path::PathBuf;
use std::process;

use missingno_gb::GameBoy;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::trace::{Profile, Tracer, Trigger};

pub fn run(
    rom_path: PathBuf,
    profile_path: PathBuf,
    output: Option<PathBuf>,
    cycles: u64,
    boot_rom: Option<Box<[u8; 256]>>,
) {
    let rom_data = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("error: failed to read ROM {}: {e}", rom_path.display());
        process::exit(1);
    });

    let profile = Profile::load(&profile_path).unwrap_or_else(|e| {
        eprintln!(
            "error: failed to load profile {}: {e}",
            profile_path.display()
        );
        process::exit(1);
    });

    let save_path = rom_path.with_extension("sav");
    let save_data = std::fs::read(&save_path).ok();
    let cartridge = Cartridge::new(rom_data, save_data);
    let title = cartridge.title().to_string();

    let boot = missingno_gb::trace::BootRom::Skip;

    let gb = GameBoy::new(cartridge, boot_rom);

    let output_path = output.unwrap_or_else(|| {
        let stem = rom_path.file_stem().unwrap().to_string_lossy();
        PathBuf::from(format!("{stem}.gbtrace"))
    });

    let mut tracer = Tracer::create(&output_path, &profile, &gb, boot).unwrap_or_else(|e| {
        eprintln!("error: failed to create trace file: {e}");
        process::exit(1);
    });
    tracer.mark_frame().unwrap();

    let mut gb = gb;
    let mut dots: u64 = 0;
    let mut frames = 0u64;
    let mut instructions = 0u64;

    let is_tcycle = profile.trigger == Trigger::Tcycle;

    eprintln!("tracing: {title}");
    eprintln!("profile: {}", profile_path.display());
    eprintln!("output: {}", output_path.display());
    eprintln!("limit: {cycles} dots");

    if is_tcycle {
        // T-cycle level tracing
        gb.cpu_mut().take_instruction_boundary();
        while dots < cycles {
            let rise = gb.step_phase();
            if let Some(pixel) = rise.pixel {
                tracer.push_pixel(pixel.shade);
            }
            let fall = gb.step_phase();
            if let Some(pixel) = fall.pixel {
                tracer.push_pixel(pixel.shade);
            }
            if rise.new_screen || fall.new_screen {
                frames += 1;
                tracer.mark_frame().unwrap();
            }
            tracer.capture(&gb).unwrap();
            tracer.advance_dot();
            dots += 1;
            if gb.cpu().at_instruction_boundary() {
                instructions += 1;
            }
        }
    } else {
        // Instruction level tracing
        while dots < cycles {
            tracer.capture(&gb).unwrap();
            let result = gb.step();
            tracer.advance(result.dots);
            dots += result.dots as u64;
            instructions += 1;
            if result.new_screen {
                frames += 1;
                tracer.mark_frame().unwrap();
            }
        }
    }

    tracer.finish().unwrap_or_else(|e| {
        eprintln!("error: failed to finalize trace: {e}");
        process::exit(1);
    });
    eprintln!("done: {instructions} instructions, {dots} dots, {frames} frames");
}
