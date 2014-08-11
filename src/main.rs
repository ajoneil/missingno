use gameboy::Gameboy;
use std::os;
use std::io::File;
use std::path::Path;

mod cartridge;
mod cpu;
mod gameboy;
mod mbc;
mod mmu;
mod rom_info;
mod video;


fn main() {
    let args = os::args();
    let filename = &args[1];
    let path = Path::new(filename.as_slice());
    let mut file = File::open(&path);
    let rom = match file.read_to_end() {
        Ok(e) => e,
        _ => fail!()
    };
    let mut gb = Gameboy::new(rom);

    gb.run();
}
