//! The BIOS socket: U2, the 8 KB ROM mapped at $0000-$1FFF ahead of the
//! cartridge. OS 7 holds the reset vector, so the console does not start
//! without it.

use missingno_core::firmware::{FirmwareImage, FirmwareNeed, FirmwareSlot, FirmwareValue};
use missingno_core::launch::{LaunchOptionDescriptor, LaunchOptionKind, LaunchValues};

pub const BIOS: &str = "colecovision-bios";

pub const BIOS_SIZE: usize = 0x2000;

pub const fn bios_slot() -> FirmwareSlot {
    FirmwareSlot {
        id: BIOS,
        label: "BIOS",
        need: FirmwareNeed::Required,
        size: BIOS_SIZE,
        images: IMAGES,
    }
}

const IMAGES: &[FirmwareImage] = &[FirmwareImage::official(
    "ntsc",
    "ColecoVision (1982)",
    "990bf1956f10207d8781b619eb74f89b00d921c8d45c95c334c16c8cceca09ad",
)];

/// The BIOS socket as a launch option, named by the slot itself.
pub fn bios_option() -> LaunchOptionDescriptor {
    let slot = bios_slot();
    LaunchOptionDescriptor {
        id: slot.id,
        label: slot.label,
        kind: LaunchOptionKind::Firmware { slot },
    }
}

/// The BIOS image the launch values supply. `Err` names the slot and what was
/// found in it — nothing included, since the console has no reset vector
/// without one.
pub fn bios_from_launch(values: &LaunchValues) -> Result<[u8; BIOS_SIZE], (&'static str, String)> {
    let refuse = |what: String| Err((BIOS, what));
    match values.firmware(BIOS) {
        None | Some(FirmwareValue::None) => refuse("no BIOS image supplied".to_owned()),
        // Only bytes boot: a frontend that named an image and never resolved
        // it has supplied nothing.
        Some(FirmwareValue::Image(_)) => refuse("image not supplied".to_owned()),
        Some(FirmwareValue::Bytes(bytes)) => match <[u8; BIOS_SIZE]>::try_from(bytes.as_slice()) {
            Ok(image) => Ok(image),
            Err(_) => refuse(format!("{}-byte image", bytes.len())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_image_is_named_once_and_states_its_hash() {
        bios_slot().check_well_formed();
    }

    #[test]
    fn an_image_of_the_wrong_content_is_not_recognised() {
        assert_eq!(bios_slot().identify(&[0u8; BIOS_SIZE]), None);
    }

    #[test]
    fn an_empty_socket_is_refused_by_name() {
        assert_eq!(
            bios_from_launch(&LaunchValues::default()).err(),
            Some((BIOS, "no BIOS image supplied".to_owned()))
        );
        let mut values = LaunchValues::default();
        values.set_firmware(BIOS, FirmwareValue::None);
        assert!(bios_from_launch(&values).is_err());
        values.set_firmware(BIOS, FirmwareValue::Image("ntsc".to_owned()));
        assert_eq!(
            bios_from_launch(&values).err(),
            Some((BIOS, "image not supplied".to_owned()))
        );
    }

    #[test]
    fn only_an_image_of_the_sockets_size_boots() {
        let mut values = LaunchValues::default();
        values.set_firmware(BIOS, FirmwareValue::Bytes(vec![0; 0x1000]));
        assert_eq!(
            bios_from_launch(&values).err(),
            Some((BIOS, "4096-byte image".to_owned()))
        );
        values.set_firmware(BIOS, FirmwareValue::Bytes(vec![0x5A; BIOS_SIZE]));
        assert_eq!(bios_from_launch(&values), Ok([0x5A; BIOS_SIZE]));
    }
}
