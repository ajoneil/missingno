use std::path::PathBuf;

use clap::{Parser, Subcommand};
use missingno_gb::BootRom;

mod app;
mod cartridge_rw;
mod headless;
mod link_cable;
mod render;
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

    /// Link cable: listen for connections on this port (BGB link protocol).
    #[arg(long, value_name = "PORT", conflicts_with = "link_connect")]
    link_listen: Option<u16>,

    /// Link cable: connect to a server at host:port (BGB link protocol).
    #[arg(long, value_name = "HOST:PORT", conflicts_with = "link_listen")]
    link_connect: Option<String>,
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

        /// Path to a boot ROM (DMG: 256 bytes, CGB: 2304 bytes).
        #[arg(long)]
        boot_rom: Option<PathBuf>,
    },
}

fn load_boot_rom(path: Option<PathBuf>) -> Option<BootRom> {
    path.map(|path| {
        let data = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("error: failed to read boot ROM {}: {e}", path.display());
            std::process::exit(1);
        });
        let len = data.len();
        match len {
            0x100 => BootRom::Dmg(data.into_boxed_slice().try_into().unwrap()),
            0x900 => BootRom::Cgb(data.into_boxed_slice().try_into().unwrap()),
            _ => {
                eprintln!("error: boot ROM must be 256 bytes (DMG) or 2304 bytes (CGB), got {len}");
                std::process::exit(1);
            }
        }
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

    let link = create_link(args.link_listen, args.link_connect);

    if args.headless {
        headless::run(args.rom_file, boot_rom, link);
        return Ok(());
    }

    app::run(args.rom_file, args.debugger, link)
}

fn create_link(
    listen: Option<u16>,
    connect: Option<String>,
) -> Option<Box<dyn missingno_gb::serial_transfer::SerialLink>> {
    if let Some(port) = listen {
        match link_cable::BgbLink::listen(port) {
            Ok(link) => return Some(Box::new(link)),
            Err(e) => {
                eprintln!("error: failed to start link cable listener: {e}");
                std::process::exit(1);
            }
        }
    }

    if let Some(addr) = connect {
        match link_cable::BgbLink::connect(&addr) {
            Ok(link) => return Some(Box::new(link)),
            Err(e) => {
                eprintln!("warning: link cable connection failed: {e}");
            }
        }
    }

    None
}
