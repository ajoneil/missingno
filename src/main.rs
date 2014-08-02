use rom_info::RomInfo;
use std::os;
use std::io::File;
use std::path::Path;

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
    let rom_info = RomInfo::new(rom.as_slice());

    println!("{}", rom_info.title());
}
