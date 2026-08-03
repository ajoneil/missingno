//! Capture a one-frame `.morepork` trace starting from a save state:
//! `cargo run -p missingno-vcs --example trace-from-state --features morepork -- <rom> <state.mpsv> <out.morepork> [ntsc|pal|secam]`

use std::process;

use missingno_vcs::TvStandard;
use missingno_vcs::debug::create_console;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(rom_path), Some(state_path), Some(out_path)) =
        (args.next(), args.next(), args.next())
    else {
        eprintln!("usage: trace-from-state <rom> <state.mpsv> <out.morepork> [ntsc|pal|secam]");
        process::exit(2);
    };
    let standard = args.next().map(|name| match name.as_str() {
        "ntsc" => TvStandard::Ntsc,
        "pal" => TvStandard::Pal,
        "secam" => TvStandard::Secam,
        other => {
            eprintln!("error: unknown TV standard {other}");
            process::exit(2);
        }
    });

    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("error: failed to read ROM {rom_path}: {e}");
        process::exit(1);
    });
    let state = std::fs::read(&state_path).unwrap_or_else(|e| {
        eprintln!("error: failed to read state {state_path}: {e}");
        process::exit(1);
    });

    let console = create_console(&rom, String::new(), standard, None, false).unwrap_or_else(|e| {
        eprintln!("error: failed to load ROM: {e:?}");
        process::exit(1);
    });
    let mut debugger = console.into_debugger();
    debugger.load_state(&state).unwrap_or_else(|e| {
        eprintln!("error: failed to load state: {e:?}");
        process::exit(1);
    });

    match debugger.capture_trace(out_path.as_ref()) {
        Some(_) => eprintln!("captured one frame to {out_path}"),
        None => {
            eprintln!("error: capture failed (no frame completed within budget)");
            process::exit(1);
        }
    }
}
