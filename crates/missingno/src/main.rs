use std::path::PathBuf;

use clap::Parser;

mod app;
mod headless;

#[derive(Parser)]
struct Args {
    rom_file: Option<PathBuf>,

    #[arg(short, long)]
    debugger: bool,

    #[arg(long)]
    headless: bool,

    /// Path to the DMG boot ROM (256 bytes). When provided, execution
    /// starts at 0x0000 and the boot ROM runs before handing control
    /// to the cartridge.
    #[arg(long)]
    boot_rom: Option<PathBuf>,
}

fn main() -> iced::Result {
    let args = Args::parse();

    let boot_rom = args.boot_rom.map(|path| {
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
    });

    if args.headless {
        headless::run(args.rom_file, boot_rom);
        return Ok(());
    }

    app::run(args.rom_file, args.debugger)
}
