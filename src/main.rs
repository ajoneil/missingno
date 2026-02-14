use std::path::PathBuf;

use clap::Parser;

mod app;

#[derive(Parser)]
struct Args {
    rom_file: Option<PathBuf>,

    #[arg(short, long)]
    debugger: bool,
}

fn main() -> iced::Result {
    let args = Args::parse();
    app::run(args.rom_file, args.debugger)
}
