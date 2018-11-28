use std::fs::File;
use std::io::Read;
use std::path::Path;

mod cartridge;
mod cpu;
mod gameboy;
mod mbc;
mod mmu;
mod rom_info;
mod video;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filename = &args[1];
    let path = Path::new(&filename);
    let mut file = File::open(&path).unwrap();
    let mut rom = Vec::new();
    file.read_to_end(&mut rom).unwrap();
    let mut gb = gameboy::Gameboy::new(rom);

    gb.run();
}
