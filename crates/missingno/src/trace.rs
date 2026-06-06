use std::path::PathBuf;
use std::process;

use missingno_gb::cartridge::Cartridge;
use missingno_gb::trace::{Profile, Tracer, Trigger};
use missingno_gb::{BootRom, Console, GameBoy, Model};
use missingno_gbc::GameBoyColor;

pub fn run(
    rom_path: PathBuf,
    profile_path: PathBuf,
    output: Option<PathBuf>,
    cycles: u64,
    boot_rom: Option<BootRom>,
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

    let output_path = output.unwrap_or_else(|| {
        let stem = rom_path.file_stem().unwrap().to_string_lossy();
        PathBuf::from(format!("{stem}.gbtrace"))
    });

    eprintln!("profile: {}", profile_path.display());
    eprintln!("output: {}", output_path.display());
    eprintln!("limit: {cycles} T-cycles");

    if cartridge.is_cgb() {
        trace_console(
            GameBoyColor::new(cartridge, boot_rom),
            &profile,
            &output_path,
            cycles,
        );
    } else {
        trace_console(
            GameBoy::new(cartridge, boot_rom),
            &profile,
            &output_path,
            cycles,
        );
    }
}

fn trace_console<M: Model>(
    mut gb: Console<M>,
    profile: &Profile,
    output_path: &PathBuf,
    cycles: u64,
) {
    let title = gb.cartridge().title().to_string();
    let boot = missingno_gb::trace::BootRom::Skip;

    let mut tracer = Tracer::create(output_path, profile, &gb, boot, M::TRACE_MODEL_NAME)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to create trace file: {e}");
            process::exit(1);
        });
    tracer.mark_frame().unwrap();

    let mut tcycles: u64 = 0;
    let mut frames = 0u64;
    let mut instructions = 0u64;

    let is_tcycle = profile.trigger == Trigger::Tcycle;

    eprintln!("tracing: {title}");

    if is_tcycle {
        // T-cycle level tracing; may overshoot the limit by one instruction.
        while tcycles < cycles {
            let result = missingno_gb::trace::step_instruction_tcycle(&mut gb, &mut tracer);
            tcycles += result.tcycles as u64;
            instructions += 1;
            if result.new_screen {
                frames += 1;
            }
        }
    } else {
        // Instruction level tracing
        while tcycles < cycles {
            tracer.capture(&gb).unwrap();
            let result = gb.step();
            tracer.advance(result.tcycles);
            tcycles += result.tcycles as u64;
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
    eprintln!("done: {instructions} instructions, {tcycles} T-cycles, {frames} frames");
}
