//! The boot ROM socket on a Game Boy Color board. The distributed image spans
//! `0x000`–`0x8FF`, the header window in its middle included, so a CGB-class
//! dump is 2304 bytes where a monochrome one is 256.

use missingno_core::firmware::{FirmwareImage, FirmwareNeed, FirmwareSlot};

/// The boot ROM the CGB-class consoles map over the cartridge.
pub const CGB_BOOT_ROM: &str = "cgb-boot-rom";

const CGB_BOOT_ROM_SIZE: usize = 0x900;

/// The boot ROM images the Game Boy Color recognises. Without one the core
/// starts at the cartridge entry point, so the slot is optional.
pub const fn boot_rom_slot() -> FirmwareSlot {
    FirmwareSlot {
        id: CGB_BOOT_ROM,
        label: "Boot ROM",
        need: FirmwareNeed::Optional,
        size: CGB_BOOT_ROM_SIZE,
        images: IMAGES,
    }
}

const IMAGES: &[FirmwareImage] = &[
    FirmwareImage::official(
        "cgb0",
        "CGB0 (earliest Game Boy Color)",
        "3a307a41689bee99a9a32ea021bf45136906c86b2e4f06c806738398e4f92e45",
    ),
    FirmwareImage::official(
        "cgb",
        "CGB (Game Boy Color)",
        "b4f2e416a35eef52cba161b159c7c8523a92594facb924b3ede0d722867c50c7",
    ),
    FirmwareImage::official(
        "cgbE",
        "CGB-E (late Game Boy Color)",
        "c56299bedd56debdbf36442238636bf5887a65c5173b33995682052353804da9",
    ),
    FirmwareImage::official(
        "agb0",
        "AGB0 (early Game Boy Advance)",
        "fe2d45405531756d87622abde6127c804bd675cb968081b2c052497a470ffeb2",
    ),
    FirmwareImage::official(
        "agb",
        "AGB (Game Boy Advance, Game Boy Advance SP)",
        "fe3cceb79930c4cb6c6f62f742c2562fd4c96b827584ef8ea89d49b387bd6860",
    ),
    FirmwareImage::open(
        "sameboy-cgb0",
        "SameBoy CGB0 (v0.16.7 to v1.0.3)",
        "SameBoy",
        "2c297b6cb762cd0a50253449fd026ae30c76f0cc30b919e2ff498bca7682eacc",
    ),
    FirmwareImage::open(
        "sameboy-cgb",
        "SameBoy CGB (v0.16.7 to v1.0.3)",
        "SameBoy",
        "f767b8e7e510a255f81328c89dba6e0c996b370e1bc86aebb8584a7da47a5bba",
    ),
    FirmwareImage::open(
        "sameboy-agb",
        "SameBoy AGB (v0.16.7 to v1.0.3)",
        "SameBoy",
        "648fd2ade35a77ce93fb4ceac754b0f0465f1eaf0db0cbef1bed55bfd8a71794",
    ),
    FirmwareImage::open(
        "sameboy-cgb-0.14",
        "SameBoy CGB (v0.14.7)",
        "SameBoy",
        "de1de7e29dac11afce1761362aef7a8345472b10022e042c163d49afcbee0b11",
    ),
    FirmwareImage::open(
        "sameboy-agb-0.14",
        "SameBoy AGB (v0.14.7)",
        "SameBoy",
        "cc4741f7c679a2980d6c7e27e6eb424318569159831edda7f94924adadd00d50",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_image_is_named_once_and_states_its_hash() {
        boot_rom_slot().check_well_formed();
    }

    /// The two Game Boy slots share no image: length alone separates them.
    #[test]
    fn no_monochrome_image_is_listed_here() {
        let colour = boot_rom_slot();
        for image in missingno_gb::firmware::boot_rom_slot().images {
            assert!(colour.image(image.id).is_none(), "{}", image.id);
        }
    }
}
