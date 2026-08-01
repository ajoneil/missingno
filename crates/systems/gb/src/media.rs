//! Game Boy family media recognition: whether a path and its contents are
//! Game Boy media, and which platform the header marks. Pure functions over
//! the cartridge header, shared by every load path (GUI, headless, trace).

use std::path::Path;

use crate::cartridge::Cartridge;

fn has_family_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gb") || e.eq_ignore_ascii_case("gbc"))
}

/// Any Game Boy family media: big enough to carry a header, and either
/// wearing the boot logo (real media must, or the console refuses to boot)
/// or a family file extension (hand-built test ROMs may skip the logo).
pub fn is_family_rom(path: &Path, rom: &[u8]) -> bool {
    rom.len() >= 0x150 && (Cartridge::peek_valid_header(rom) || has_family_extension(path))
}

/// Game Boy platform media: everything the family claims except CGB-required
/// cartridges — dual-mode media belongs here, even though it boots enhanced.
pub fn is_gb_rom(path: &Path, rom: &[u8]) -> bool {
    is_family_rom(path, rom) && !Cartridge::peek_cgb_only(rom)
}

/// Game Boy Color platform media: cartridges the header marks CGB-required.
pub fn is_gbc_rom(path: &Path, rom: &[u8]) -> bool {
    is_family_rom(path, rom) && Cartridge::peek_cgb_only(rom)
}

pub fn title_from_rom(rom: &[u8]) -> Option<String> {
    let title = Cartridge::peek_title(rom);
    (!title.is_empty()).then_some(title)
}
