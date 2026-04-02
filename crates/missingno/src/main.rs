use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod app;
mod headless;
mod trace;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    rom_file: Option<PathBuf>,

    #[arg(short, long)]
    debugger: bool,

    #[arg(long)]
    headless: bool,

    /// Path to the DMG boot ROM (256 bytes).
    #[arg(long)]
    boot_rom: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Dump a gbtrace file for a ROM.
    Trace {
        /// Path to the ROM file.
        rom: PathBuf,

        /// Path to the gbtrace profile TOML file.
        #[arg(short, long)]
        profile: PathBuf,

        /// Output file path. Defaults to <rom_stem>.gbtrace.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Number of T-cycles (dots) to trace.
        #[arg(short, long, default_value = "70224")]
        cycles: u64,

        /// Path to the DMG boot ROM (256 bytes).
        #[arg(long)]
        boot_rom: Option<PathBuf>,
    },
}

fn load_boot_rom(path: Option<PathBuf>) -> Option<Box<[u8; 256]>> {
    path.map(|path| {
        let data = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("error: failed to read boot ROM {}: {e}", path.display());
            std::process::exit(1);
        });
        let len = data.len();
        let boxed: Box<[u8; 256]> = data.into_boxed_slice().try_into().unwrap_or_else(|_| {
            eprintln!("error: boot ROM must be exactly 256 bytes (got {len})");
            std::process::exit(1);
        });
        boxed
    })
}

fn main() -> iced::Result {
    let args = Args::parse();

    if let Some(command) = args.command {
        match command {
            Command::Trace {
                rom,
                profile,
                output,
                cycles,
                boot_rom,
            } => {
                trace::run(rom, profile, output, cycles, load_boot_rom(boot_rom));
            }
        }
        return Ok(());
    }

    let boot_rom = load_boot_rom(args.boot_rom);

    if args.headless {
        headless::run(args.rom_file, boot_rom);
        return Ok(());
    }

    app::run(args.rom_file, args.debugger)
}
