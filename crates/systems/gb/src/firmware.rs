//! The boot ROM socket on a Game Boy board, and the 256-byte images that go in
//! it. Every model of the monochrome family maps its own dump; a reader tells
//! them apart by content, not by the name a file was saved under.

use missingno_core::firmware::{FirmwareImage, FirmwareNeed, FirmwareSlot};

/// The boot ROM the DMG-class consoles map over the cartridge.
pub const DMG_BOOT_ROM: &str = "dmg-boot-rom";

/// The size every monochrome boot ROM is: `0x0000`–`0x00FF`.
const DMG_BOOT_ROM_SIZE: usize = 0x100;

/// The boot ROM images the Game Boy recognises. Without one the core starts at
/// the cartridge entry point, so the slot is optional.
pub const fn boot_rom_slot() -> FirmwareSlot {
    FirmwareSlot {
        id: DMG_BOOT_ROM,
        label: "Game Boy boot ROM",
        need: FirmwareNeed::Optional,
        size: DMG_BOOT_ROM_SIZE,
        images: IMAGES,
    }
}

const IMAGES: &[FirmwareImage] = &[
    FirmwareImage::official(
        "dmg0",
        "DMG0 (launch Game Boy)",
        "26e71cf01e301e5dc40e987cd2ecbf6d0276245890ac829db2a25323da86818e",
    ),
    FirmwareImage::official(
        "dmg",
        "DMG (Game Boy)",
        "cf053eccb4ccafff9e67339d4e78e98dce7d1ed59be819d2a1ba2232c6fce1c7",
    ),
    FirmwareImage::official(
        "mgb",
        "MGB (Game Boy Pocket, Game Boy Light)",
        "a8cb5f4f1f16f2573ed2ecd8daedb9c5d1dd2c30a481f9b179b5d725d95eafe2",
    ),
    FirmwareImage::official(
        "sgb",
        "SGB (Super Game Boy)",
        "0e4ddff32fc9d1eeaae812a157dd246459b00c9e14f2f61751f661f32361e360",
    ),
    FirmwareImage::official(
        "sgb2",
        "SGB2 (Super Game Boy 2)",
        "fd243c4fb27008986316ce3df29e9cfbcdc0cd52704970555a8bb76edbec3988",
    ),
    FirmwareImage::open(
        "sameboy-dmg",
        "SameBoy DMG (v0.16.7 to v1.0.3)",
        "SameBoy",
        "6f64da4cecd7e54e2f928eb3e3ba7810a7a567d0d247cc71737d1771e073a916",
    ),
    FirmwareImage::open(
        "sameboy-mgb",
        "SameBoy MGB (v0.16.7 to v1.0.3)",
        "SameBoy",
        "12eb4b96f8c2c19cd51c209af1f0a5aa19e01f886114ae848258a592f408ef4e",
    ),
    FirmwareImage::open(
        "sameboy-sgb",
        "SameBoy SGB (v0.16.7 to v1.0.3)",
        "SameBoy",
        "b60d493a7944ccf74c81f1e7b6bf38c2c7029e296648ea0e74cb22b10dd1fcb8",
    ),
    FirmwareImage::open(
        "sameboy-sgb2",
        "SameBoy SGB2 (v0.16.7 to v1.0.3)",
        "SameBoy",
        "8a65465a9da7ec657726a671da9a85963cf19d2c975e499aea6b97eb37e0b6ea",
    ),
    FirmwareImage::open(
        "sameboy-dmg-0.14",
        "SameBoy DMG (v0.14.7)",
        "SameBoy",
        "213f95deeb4e1e61eab907ac283e1083a3a14efd8384149044724acc9879b7a6",
    ),
    FirmwareImage::open(
        "sameboy-sgb-0.14",
        "SameBoy SGB (v0.14.7)",
        "SameBoy",
        "cce9a42946d8054ba701c94fbec3684a37688ad53e2c1bed1046c1afae72f40a",
    ),
    FirmwareImage::open(
        "sameboy-sgb2-0.14",
        "SameBoy SGB2 (v0.14.7)",
        "SameBoy",
        "b701a4babf51d783e82befde4dae270a2f22acb70656f8229bdd6589c0f6d06b",
    ),
    FirmwareImage::open(
        "bootix-dmg-1.2",
        "Bootix DMG (v1.2)",
        "Bootix",
        "c313435280dda8ccfae0786c7159aab791a00930605101c647794f64bfa17b5e",
    ),
    FirmwareImage::open(
        "bootix-mgb-1.2",
        "Bootix MGB (v1.2)",
        "Bootix",
        "615fe89dad2d6e2ed8828e9a80b05c9cf475ba28f02237988167371239a6af66",
    ),
    FirmwareImage::open(
        "bootix-dmg-1.1",
        "Bootix DMG (v1.1)",
        "Bootix",
        "1ece2a82d82de7fc106ee6a608e8dee268c0f2a6eac8df4a7584145295b65fd0",
    ),
    FirmwareImage::open(
        "bootix-dmg-1.0",
        "Bootix DMG (v1.0)",
        "Bootix",
        "b2cdb689994460079a1f6207b9321e4e32901bf086f644fc0b66c20970e4efb8",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_image_is_named_once_and_states_its_hash() {
        boot_rom_slot().check_well_formed();
    }

    #[test]
    fn an_image_of_the_wrong_content_is_not_recognised() {
        assert_eq!(boot_rom_slot().identify(&[0u8; DMG_BOOT_ROM_SIZE]), None);
    }
}
