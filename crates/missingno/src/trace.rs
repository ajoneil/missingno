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

    let output_path = output.unwrap_or_else(|| {
        let stem = rom_path.file_stem().unwrap().to_string_lossy();
        PathBuf::from(format!("{stem}.gbtrace"))
    });

    eprintln!("profile: {}", profile_path.display());
    eprintln!("output: {}", output_path.display());

    if rom_data.len() >= 4 && &rom_data[0..4] == b"NES\x1a" {
        #[cfg(feature = "nes")]
        {
            eprintln!("limit: {cycles} CPU cycles");
            trace_nes(&rom_data, &profile, &output_path, cycles);
            return;
        }
        #[cfg(not(feature = "nes"))]
        {
            eprintln!("error: iNES ROM, but this build has no NES support (--features nes)");
            process::exit(1);
        }
    }

    let is_a26 = rom_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("a26"));
    if is_a26 || matches!(rom_data.len(), 0x800 | 0x1000) {
        // Bare VCS ROM sizes cannot collide with Game Boy ROMs (those
        // start at 32 KiB).
        #[cfg(feature = "vcs")]
        {
            eprintln!("limit: {cycles} CPU cycles");
            trace_vcs(&rom_data, &profile, &output_path, cycles);
            return;
        }
        #[cfg(not(feature = "vcs"))]
        {
            eprintln!("error: VCS ROM, but this build has no VCS support (--features vcs)");
            process::exit(1);
        }
    }

    eprintln!("limit: {cycles} T-cycles");
    let save_path = rom_path.with_extension("sav");
    let save_data = std::fs::read(&save_path).ok();
    let cartridge = Cartridge::new(rom_data, save_data);

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

#[cfg(feature = "nes")]
fn trace_nes(rom: &[u8], profile: &Profile, output_path: &PathBuf, cycle_limit: u64) {
    use missingno_nes::console::Nes;
    use missingno_nes::trace::{Tracer, step_instruction_counted};

    let mut nes = Nes::new(rom).unwrap_or_else(|e| {
        eprintln!("error: failed to load NES ROM: {e:?}");
        process::exit(1);
    });
    let mut tracer = Tracer::create(output_path, profile, rom).unwrap_or_else(|e| {
        eprintln!("error: failed to create trace file: {e}");
        process::exit(1);
    });

    let per_cycle = profile.trigger == Trigger::Cycle;
    let mut total_cycles: u64 = 0;
    let mut instructions = 0u64;
    let mut frames = 0u64;
    let mut last_cycles = 0u16;

    while total_cycles < cycle_limit {
        // Pre-execution state, with the previous step's cycle cost.
        tracer.capture(&nes, last_cycles).unwrap();
        if per_cycle {
            nes.step_cycle();
            last_cycles = 1;
        } else {
            last_cycles = step_instruction_counted(&mut nes);
            instructions += 1;
        }
        total_cycles += last_cycles as u64;
        if let Some(frame) = nes.take_frame() {
            frames += 1;
            tracer.mark_frame(Some(&frame)).unwrap();
        }
    }

    tracer.finish().unwrap_or_else(|e| {
        eprintln!("error: failed to finalize trace: {e}");
        process::exit(1);
    });
    if per_cycle {
        eprintln!("done: {total_cycles} cycles, {frames} frames");
    } else {
        eprintln!("done: {instructions} instructions, {total_cycles} cycles, {frames} frames");
    }
}

#[cfg(feature = "vcs")]
fn trace_vcs(rom: &[u8], profile: &Profile, output_path: &PathBuf, cycle_limit: u64) {
    use missingno_vcs::console::Vcs;
    use missingno_vcs::trace::{Tracer, step_instruction_counted};

    let mut vcs = Vcs::new(rom, missingno_vcs::TvStandard::Ntsc).unwrap_or_else(|e| {
        eprintln!("error: failed to load VCS ROM: {e:?}");
        process::exit(1);
    });
    let mut tracer =
        Tracer::create(output_path, profile, rom, vcs.tv_standard()).unwrap_or_else(|e| {
            eprintln!("error: failed to create trace file: {e}");
            process::exit(1);
        });

    let per_cycle = profile.trigger == Trigger::Cycle;
    let mut total_cycles: u64 = 0;
    let mut instructions = 0u64;
    let mut frames = 0u64;
    let mut last_cycles = 0u16;

    while total_cycles < cycle_limit {
        // Pre-execution state, with the previous step's cycle cost.
        tracer.capture(&vcs, last_cycles).unwrap();
        if per_cycle {
            vcs.step_cpu_cycle();
            last_cycles = 1;
        } else {
            last_cycles = step_instruction_counted(&mut vcs);
            instructions += 1;
        }
        total_cycles += last_cycles as u64;
        if let Some(frame) = vcs.take_frame() {
            frames += 1;
            tracer.mark_frame(Some(&frame)).unwrap();
        }
    }

    tracer.finish().unwrap_or_else(|e| {
        eprintln!("error: failed to finalize trace: {e}");
        process::exit(1);
    });
    if per_cycle {
        eprintln!("done: {total_cycles} cycles, {frames} frames");
    } else {
        eprintln!("done: {instructions} instructions, {total_cycles} cycles, {frames} frames");
    }
}
