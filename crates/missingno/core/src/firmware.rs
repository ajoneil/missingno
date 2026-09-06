//! The firmware sockets on a board, and the images a core recognises in them.
//!
//! A console that maps a program of its own before the cartridge — a Game Boy
//! boot ROM, a ColecoVision BIOS — states that socket as a [`FirmwareSlot`]
//! listing the images it knows by content. Nothing here reads a file: a core
//! *states* what it accepts and a frontend supplies the bytes, so identity is
//! the SHA-256 of an image rather than the name someone saved it under.

use crate::launch::{LaunchOptionDescriptor, LaunchOptionKind};

/// One firmware image a core knows by content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareImage {
    pub id: &'static str,
    pub label: &'static str,
    /// 64 lower-hex characters.
    pub sha256: &'static str,
    pub origin: FirmwareOrigin,
}

impl FirmwareImage {
    /// A dump from the console model `label` names.
    pub const fn official(id: &'static str, label: &'static str, sha256: &'static str) -> Self {
        FirmwareImage {
            id,
            label,
            sha256,
            origin: FirmwareOrigin::Official,
        }
    }

    /// A reimplementation `project` publishes.
    pub const fn open(
        id: &'static str,
        label: &'static str,
        project: &'static str,
        sha256: &'static str,
    ) -> Self {
        FirmwareImage {
            id,
            label,
            sha256,
            origin: FirmwareOrigin::Open { project },
        }
    }
}

/// Where an image came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareOrigin {
    /// Dumped from the console model the image's label names.
    Official,
    /// A reimplementation `project` publishes.
    Open { project: &'static str },
}

/// Whether a machine starts without the slot filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareNeed {
    /// The machine does not start without an image.
    Required,
    /// Without one the core starts where the firmware would have handed over.
    Optional,
}

/// One firmware a core maps at launch, and the images it recognises for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareSlot {
    pub id: &'static str,
    pub label: &'static str,
    pub need: FirmwareNeed,
    /// Every image this socket takes is this many bytes.
    pub size: usize,
    /// In the core's preferred order, official dumps first.
    pub images: &'static [FirmwareImage],
}

/// What a caller fills a firmware socket with.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FirmwareValue {
    /// Nothing, which only a [`FirmwareNeed::Optional`] socket starts without.
    None,
    /// An image the slot lists, by id; a frontend turns it into the bytes.
    Image(String),
    /// The image itself, which is all a core reads.
    Bytes(Vec<u8>),
}

impl FirmwareSlot {
    /// The image this slot lists under `id`.
    pub fn image(&self, id: &str) -> Option<&'static FirmwareImage> {
        self.images.iter().find(|image| image.id == id)
    }

    /// The image `bytes` are, if this slot lists one with that content.
    pub fn identify(&self, bytes: &[u8]) -> Option<&'static FirmwareImage> {
        if bytes.len() != self.size {
            return None;
        }
        let hash = sha256_hex(bytes);
        self.images.iter().find(|image| image.sha256 == hash)
    }

    /// Panics unless every image is named once and states a lower-hex SHA-256.
    /// A core's own socket test is a call to this.
    #[track_caller]
    pub fn check_well_formed(&self) {
        let mut ids: Vec<&str> = self.images.iter().map(|image| image.id).collect();
        let published = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), published, "{}: an id names two images", self.id);
        for image in self.images {
            assert_eq!(image.sha256.len(), 64, "{}", image.id);
            assert!(
                image
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{}",
                image.id
            );
        }
    }
}

/// Every firmware socket among `descriptors`, in the order they publish them.
pub fn firmware_slots(
    descriptors: &[LaunchOptionDescriptor],
) -> impl Iterator<Item = &FirmwareSlot> {
    descriptors.iter().filter_map(|option| match &option.kind {
        LaunchOptionKind::Firmware { slot } => Some(slot),
        _ => None,
    })
}

/// An image's identity: the lower-hex SHA-256 of its bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::machine::hex_digest(&crate::machine::rom_fingerprint(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic images: the hashes are of the four bytes each test hands in.
    const IMAGES: &[FirmwareImage] = &[
        FirmwareImage::official(
            "ones",
            "All ones",
            "27ecd0a598e76f8a2fd264d427df0a119903e8eae384e478902541756f089dd1",
        ),
        FirmwareImage::open(
            "zeroes",
            "All zeroes",
            "test",
            "df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119",
        ),
    ];

    const SLOT: FirmwareSlot = FirmwareSlot {
        id: "boot-rom",
        label: "Boot ROM",
        need: FirmwareNeed::Optional,
        size: 4,
        images: IMAGES,
    };

    #[test]
    fn a_known_hash_names_the_image() {
        assert_eq!(SLOT.identify(&[1u8; 4]).map(|image| image.id), Some("ones"));
        assert_eq!(
            SLOT.identify(&[0u8; 4]).map(|image| image.id),
            Some("zeroes")
        );
        assert_eq!(SLOT.identify(&[2u8; 4]), None);
    }

    #[test]
    fn an_image_of_another_length_is_not_this_slots() {
        assert_eq!(SLOT.identify(&[1u8; 8]), None);
    }

    #[test]
    fn an_image_is_looked_up_by_id() {
        assert_eq!(
            SLOT.image("zeroes").map(|image| image.label),
            Some("All zeroes")
        );
        assert_eq!(SLOT.image("agb"), None);
    }

    #[test]
    fn a_well_formed_slot_passes_its_own_check() {
        SLOT.check_well_formed();
    }

    #[test]
    fn the_hash_is_lower_hex() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn only_the_firmware_options_are_sockets() {
        let descriptors = [
            LaunchOptionDescriptor {
                id: "overdump",
                label: "Overdump",
                kind: LaunchOptionKind::Toggle,
            },
            LaunchOptionDescriptor {
                id: SLOT.id,
                label: SLOT.label,
                kind: LaunchOptionKind::Firmware { slot: SLOT },
            },
        ];
        let found: Vec<&str> = firmware_slots(&descriptors).map(|slot| slot.id).collect();
        assert_eq!(found, ["boot-rom"]);
    }
}
