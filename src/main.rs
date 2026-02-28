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
}

fn main() -> iced::Result {
    let args = Args::parse();

    if args.headless {
        headless::run(args.rom_file);
        return Ok(());
    }

    app::run(args.rom_file, args.debugger)
}
