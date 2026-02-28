use std::path::PathBuf;
use std::process;

use missingno_core::debugger::Debugger;
use missingno_core::game_boy::GameBoy;
use missingno_core::game_boy::cartridge::Cartridge;

pub fn run(rom_path: Option<PathBuf>) {
    let rom_path = rom_path.unwrap_or_else(|| {
        eprintln!("error: --headless requires a ROM file");
        process::exit(1);
    });

    let rom_data = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("error: failed to read {}: {e}", rom_path.display());
        process::exit(1);
    });

    let save_path = rom_path.with_extension("sav");
    let save_data = std::fs::read(&save_path).ok();

    let cartridge = Cartridge::new(rom_data, save_data);
    let title = cartridge.title().to_string();
    let game_boy = GameBoy::new(cartridge);
    let _debugger = Debugger::new(game_boy);

    eprintln!("headless debugger ready: {title}");
}
