use gameboy::Gameboy;
use std::os;
use std::io::File;
use std::path::Path;

mod cartridge;
mod cpu;
mod gameboy;
mod mbc;
mod rom_info;


fn main() {
    let args = os::args();
    let filename = &args[1];
    let path = Path::new(filename.as_slice());
    let mut file = File::open(&path);
    let rom = match file.read_to_end() {
        Ok(e) => e,
        _ => fail!()
    };
    let gb = Gameboy::new(rom);
}
