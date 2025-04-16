use std::{env, path::PathBuf};

mod app;
mod debugger;
mod emulator;

fn main() -> iced::Result {
    let rom_path = if let Some(path) = env::args().nth(1) {
        Some(PathBuf::from(path))
    } else {
        None
    };

    app::run(rom_path)
}
