use std::path::PathBuf;

use clap::{Parser, Subcommand};
use missingno_core::firmware::FirmwareValue;
use missingno_core::launch::LaunchValues;

mod app;
mod cartridge_rw;
mod link_cable;
mod patch;
mod printer;
mod sram;
mod trace;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    rom_file: Option<PathBuf>,

    #[arg(short, long)]
    debugger: bool,

    /// Path to a firmware image to map for this run, such as a Game Boy boot
    /// ROM. The socket it fills is the one its length fits.
    #[arg(long)]
    boot_rom: Option<PathBuf>,

    /// Link cable: listen for connections on this port (BGB link protocol).
    #[arg(long, value_name = "PORT", conflicts_with = "link_connect")]
    link_listen: Option<u16>,

    /// Link cable: connect to a server at host:port (BGB link protocol).
    #[arg(long, value_name = "HOST:PORT", conflicts_with = "link_listen")]
    link_connect: Option<String>,

    /// Publish a UI-automation socket for this run, without persisting the
    /// setting.
    #[arg(long)]
    allow_ui_automation: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Dump a morepork file for a ROM.
    Trace {
        /// Path to the ROM file.
        rom: PathBuf,

        /// Path to the morepork profile TOML file.
        #[arg(short, long)]
        profile: PathBuf,

        /// Output file path. Defaults to <rom_stem>.morepork.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Number of T-cycles (dots) to trace.
        #[arg(short, long, default_value = "70224")]
        cycles: u64,

        /// Path to a firmware image to map for this run, such as a Game Boy
        /// boot ROM.
        #[arg(long)]
        boot_rom: Option<PathBuf>,
    },
}

/// The socket a named firmware image fills, classified once against every
/// socket the registered families state: a recognised image names its own, and
/// anything else fits the socket that takes its length.
fn firmware_image(path: Option<PathBuf>) -> Option<(&'static str, FirmwareValue)> {
    let path = path?;
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!(
            "error: failed to read firmware image {}: {e}",
            path.display()
        );
        std::process::exit(1);
    });
    let slots = app::system::firmware_slots();
    let fitted = slots
        .iter()
        .find(|slot| slot.identify(&bytes).is_some())
        .or_else(|| slots.iter().find(|slot| slot.size == bytes.len()));
    match fitted {
        Some(slot) => Some((slot.id, FirmwareValue::Bytes(bytes))),
        None => {
            let mut sizes: Vec<usize> = slots.iter().map(|slot| slot.size).collect();
            sizes.sort_unstable();
            sizes.dedup();
            let named: Vec<String> = sizes.iter().map(|size| format!("{size} bytes")).collect();
            eprintln!(
                "error: {} is {} bytes; a firmware image is {}",
                path.display(),
                bytes.len(),
                named.join(" or ")
            );
            std::process::exit(1);
        }
    }
}

fn main() -> iced::Result {
    // Under gamescope, winit's X11 backend guesses a huge scale factor from the
    // panel's physical size; pin 1:1 unless the user already overrode it.
    if app::running_under_gamescope() && std::env::var_os("WINIT_X11_SCALE_FACTOR").is_none() {
        // SAFETY: no other threads exist yet.
        unsafe { std::env::set_var("WINIT_X11_SCALE_FACTOR", "1") };
    }

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
                let mut launch = LaunchValues::default();
                if let Some((slot, image)) = firmware_image(boot_rom) {
                    launch.set_firmware(slot, image);
                }
                trace::run(rom, profile, output, cycles, launch);
            }
        }
        return Ok(());
    }

    let cli_firmware = firmware_image(args.boot_rom);

    let link = create_link(args.link_listen, args.link_connect);

    app::run(
        args.rom_file,
        args.debugger,
        link,
        cli_firmware,
        args.allow_ui_automation,
    )
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
