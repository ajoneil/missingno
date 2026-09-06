//! The firmware sockets on a board, and the images a core recognises in them.
//!
//! A console that maps a program of its own before the cartridge — a Game Boy
//! boot ROM, a ColecoVision BIOS — states that socket as a [`FirmwareSlot`]
//! listing the images it knows by content. Nothing here reads a file: a core
//! *states* what it accepts and a frontend supplies the bytes, so identity is
//! the SHA-256 of an image rather than the name someone saved it under.

/// One firmware image a core knows by content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareImage {
    pub id: &'static str,
    pub label: &'static str,
    pub size: usize,
    /// 64 lower-hex characters.
    pub sha256: &'static str,
    pub origin: FirmwareOrigin,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareSlot {
    pub id: &'static str,
    pub label: &'static str,
    pub need: FirmwareNeed,
    /// In the core's preferred order, official dumps first.
    pub images: Vec<FirmwareImage>,
}

/// The value that states "no image" for an [`FirmwareNeed::Optional`] slot.
pub const FIRMWARE_NONE: &str = "none";

impl FirmwareSlot {
    /// The image this slot lists under `id`.
    pub fn image(&self, id: &str) -> Option<&FirmwareImage> {
        self.images.iter().find(|image| image.id == id)
    }

    /// The image `bytes` are, if this slot lists one with that content.
    pub fn identify(&self, bytes: &[u8]) -> Option<&FirmwareImage> {
        let candidates: Vec<&FirmwareImage> = self
            .images
            .iter()
            .filter(|image| image.size == bytes.len())
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let hash = sha256_hex(bytes);
        candidates.into_iter().find(|image| image.sha256 == hash)
    }
}

/// An image's identity: the lower-hex SHA-256 of its bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic images: the hashes are of the four bytes each test hands in.
    fn slot() -> FirmwareSlot {
        FirmwareSlot {
            id: "boot-rom",
            label: "Boot ROM",
            need: FirmwareNeed::Optional,
            images: vec![
                FirmwareImage {
                    id: "ones",
                    label: "All ones",
                    size: 4,
                    sha256: "27ecd0a598e76f8a2fd264d427df0a119903e8eae384e478902541756f089dd1",
                    origin: FirmwareOrigin::Official,
                },
                FirmwareImage {
                    id: "zeroes",
                    label: "All zeroes",
                    size: 4,
                    sha256: "df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119",
                    origin: FirmwareOrigin::Open { project: "test" },
                },
            ],
        }
    }

    #[test]
    fn a_known_hash_names_the_image() {
        let slot = slot();
        assert_eq!(slot.identify(&[1u8; 4]).map(|image| image.id), Some("ones"));
        assert_eq!(
            slot.identify(&[0u8; 4]).map(|image| image.id),
            Some("zeroes")
        );
        assert_eq!(slot.identify(&[2u8; 4]), None);
    }

    #[test]
    fn an_image_of_another_length_is_not_this_slots() {
        assert_eq!(slot().identify(&[1u8; 8]), None);
    }

    #[test]
    fn an_image_is_looked_up_by_id() {
        let slot = slot();
        assert_eq!(
            slot.image("zeroes").map(|image| image.label),
            Some("All zeroes")
        );
        assert_eq!(slot.image("agb"), None);
    }

    #[test]
    fn the_hash_is_lower_hex() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
