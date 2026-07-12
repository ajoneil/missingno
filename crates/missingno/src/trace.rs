use std::path::{Path, PathBuf};
use std::process;

use missingno_gb::trace::{Profile, Tracer, Trigger};
use missingno_gb::{BootRom, Console, GameBoy, Model};
use missingno_gbc::GameBoyColor;

use crate::app::system::{self, TraceRequest, gb::GbLaunch};

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

    let Some(family) = system::family_for(&rom_path, &rom_data) else {
        eprintln!("error: unsupported ROM: {}", rom_path.display());
        process::exit(1);
    };
    let Some(trace) = family.trace else {
        eprintln!("error: {} has no trace backend", family.platform_name);
        process::exit(1);
    };
    trace(TraceRequest {
        rom: &rom_data,
        rom_path: &rom_path,
        profile: &profile,
        output: &output_path,
        cycles,
        boot_rom,
    });
}

pub(crate) fn trace_gb(request: TraceRequest) {
    eprintln!("limit: {} T-cycles", request.cycles);
    let save_data = std::fs::read(request.rom_path.with_extension("sav")).ok();

    struct Trace<'a> {
        profile: &'a Profile,
        output: &'a Path,
        cycles: u64,
    }
    impl GbLaunch for Trace<'_> {
        type Output = ();
        fn dmg(self, console: GameBoy) {
            trace_console(console, self.profile, self.output, self.cycles);
        }
        fn cgb(self, console: GameBoyColor) {
            trace_console(console, self.profile, self.output, self.cycles);
        }
    }
    system::gb::launch(
        request.rom.to_vec(),
        save_data,
        request.boot_rom,
        None,
        Trace {
            profile: request.profile,
            output: request.output,
            cycles: request.cycles,
        },
    );
}

fn trace_console<M: Model>(mut gb: Console<M>, profile: &Profile, output_path: &Path, cycles: u64) {
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
pub(crate) fn trace_nes(request: TraceRequest) {
    use missingno_nes::console::Nes;
    use missingno_nes::trace::{Tracer, step_instruction_counted};

    let (rom, profile, output_path, cycle_limit) =
        (request.rom, request.profile, request.output, request.cycles);
    eprintln!("limit: {cycle_limit} CPU cycles");

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

pub(crate) fn trace_vcs(request: TraceRequest) {
    use missingno_vcs::console::Vcs;
    use missingno_vcs::trace::{Tracer, step_instruction_counted};

    let (rom, profile, output_path, cycle_limit) =
        (request.rom, request.profile, request.output, request.cycles);
    eprintln!("limit: {cycle_limit} CPU cycles");

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
