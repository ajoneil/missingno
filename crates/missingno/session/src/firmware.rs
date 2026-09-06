//! The firmware folder: the files a user dropped in it, matched against the
//! images the loaded cores recognise.
//!
//! A core states its sockets ([`FirmwareSlot`]) and a caller states which image
//! to fill one with by id; this is where an id becomes bytes. Identity is
//! content, so a file's name is only what it is displayed as — a dump saved as
//! `boot.bin` is recognised as readily as one saved under its model's name.

use std::path::{Path, PathBuf};

use missingno_core::firmware::{
    FIRMWARE_NONE, FirmwareImage, FirmwareNeed, FirmwareSlot, sha256_hex,
};
use missingno_core::launch::{LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues};

/// Files larger than this are not firmware, and are not read to find out.
const LARGEST_IMAGE: u64 = 1 << 20;

/// One recognised image, and the file it was found in.
pub struct PresentImage {
    pub slot: &'static str,
    pub image: FirmwareImage,
    pub path: PathBuf,
}

/// What a firmware folder holds, read against a set of slots.
pub struct FirmwareLibrary {
    dir: PathBuf,
    present: Vec<PresentImage>,
    unrecognised: Vec<PathBuf>,
}

/// Why a launch cannot be given the firmware it asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirmwareRefusal {
    /// A required socket with nothing to fill it.
    Missing {
        slot: &'static str,
        label: &'static str,
        dir: PathBuf,
    },
    /// A recognised image, but no file in the folder holds it.
    NotInFolder {
        slot: &'static str,
        image: String,
        dir: PathBuf,
    },
    /// A required socket cannot be left empty.
    NoneNotAllowed { slot: &'static str },
    /// An image id the slot does not list.
    UnknownImage { slot: &'static str, image: String },
}

impl std::fmt::Display for FirmwareRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FirmwareRefusal::Missing { label, dir, .. } => {
                write!(f, "this console needs a {label} in {}", dir.display())
            }
            FirmwareRefusal::NotInFolder { slot, image, dir } => write!(
                f,
                "{slot}: no file in {} holds the {image} image",
                dir.display()
            ),
            FirmwareRefusal::NoneNotAllowed { slot } => {
                write!(f, "{slot}: this console does not start without one")
            }
            FirmwareRefusal::UnknownImage { slot, image } => {
                write!(f, "{slot}: no such image \"{image}\"")
            }
        }
    }
}

impl FirmwareLibrary {
    /// Where firmware lives when a caller names no folder of its own.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("missingno").join("firmware"))
    }

    /// Read `dir`, naming every file that holds an image one of `slots` lists.
    pub fn scan(dir: PathBuf, slots: &[FirmwareSlot]) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.path())
            .collect();
        paths.sort();

        let mut present: Vec<PresentImage> = Vec::new();
        let mut unrecognised = Vec::new();
        for path in paths {
            let Some(bytes) = read_candidate(&path) else {
                unrecognised.push(path);
                continue;
            };
            match identify(slots, &bytes) {
                Some((slot, image)) => {
                    let already = present
                        .iter()
                        .any(|found| found.slot == slot && found.image.id == image.id);
                    if !already {
                        present.push(PresentImage {
                            slot,
                            image: image.clone(),
                            path,
                        });
                    }
                }
                None => unrecognised.push(path),
            }
        }
        FirmwareLibrary {
            dir,
            present,
            unrecognised,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Files in the folder that hold no image any slot lists.
    pub fn unrecognised(&self) -> &[PathBuf] {
        &self.unrecognised
    }

    /// The images present for `slot`, in the order the slot lists them.
    pub fn present(&self, slot: &FirmwareSlot) -> Vec<&PresentImage> {
        slot.images
            .iter()
            .filter_map(|image| self.find(slot.id, image.id))
            .collect()
    }

    pub fn find(&self, slot_id: &str, image_id: &str) -> Option<&PresentImage> {
        self.present
            .iter()
            .find(|found| found.slot == slot_id && found.image.id == image_id)
    }

    /// What a slot resolves to when nobody chose: the stated `default` if the
    /// folder holds it, else the first present image for a socket that must be
    /// filled, else nothing.
    pub fn automatic(&self, slot: &FirmwareSlot, default: Option<&str>) -> Option<&PresentImage> {
        if let Some(found) = default.and_then(|id| self.find(slot.id, id)) {
            return Some(found);
        }
        match slot.need {
            FirmwareNeed::Required => self.present(slot).into_iter().next(),
            FirmwareNeed::Optional => None,
        }
    }

    /// Turn every firmware choice in `values` into the bytes a core reads. A
    /// caller that supplied a file outright keeps it.
    pub fn supply(
        &self,
        descriptors: &[LaunchOptionDescriptor],
        values: &mut LaunchValues,
    ) -> Result<(), FirmwareRefusal> {
        for descriptor in descriptors {
            let LaunchOptionKind::Firmware { slot } = &descriptor.kind else {
                continue;
            };
            match values.value(slot.id) {
                None => {
                    if slot.need == FirmwareNeed::Required {
                        return Err(FirmwareRefusal::Missing {
                            slot: slot.id,
                            label: slot.label,
                            dir: self.dir.clone(),
                        });
                    }
                }
                Some(LaunchValue::File(_)) => {}
                Some(LaunchValue::Choice(chosen)) if chosen == FIRMWARE_NONE => {
                    if slot.need == FirmwareNeed::Required {
                        return Err(FirmwareRefusal::NoneNotAllowed { slot: slot.id });
                    }
                    values.clear(slot.id);
                }
                Some(LaunchValue::Choice(chosen)) => {
                    let chosen = chosen.clone();
                    let bytes = self.image_bytes(slot, &chosen)?;
                    values.set_file(slot.id, bytes);
                }
                Some(other) => {
                    return Err(FirmwareRefusal::UnknownImage {
                        slot: slot.id,
                        image: not_an_image(other).to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn image_bytes(&self, slot: &FirmwareSlot, image: &str) -> Result<Vec<u8>, FirmwareRefusal> {
        if slot.image(image).is_none() {
            return Err(FirmwareRefusal::UnknownImage {
                slot: slot.id,
                image: image.to_owned(),
            });
        }
        let not_in_folder = || FirmwareRefusal::NotInFolder {
            slot: slot.id,
            image: image.to_owned(),
            dir: self.dir.clone(),
        };
        let found = self.find(slot.id, image).ok_or_else(not_in_folder)?;
        std::fs::read(&found.path).map_err(|_| not_in_folder())
    }
}

/// What a value that names no firmware image is instead.
fn not_an_image(value: &LaunchValue) -> &'static str {
    match value {
        LaunchValue::Flags(_) => "a set of flags",
        LaunchValue::Toggle(_) => "a toggle",
        LaunchValue::Board(_) => "a board",
        LaunchValue::Choice(_) | LaunchValue::File(_) => "an image",
    }
}

/// A file's contents, unless it is too big to be firmware.
fn read_candidate(path: &Path) -> Option<Vec<u8>> {
    let size = std::fs::metadata(path).ok()?.len();
    (size <= LARGEST_IMAGE)
        .then(|| std::fs::read(path).ok())
        .flatten()
}

/// The slot and image `bytes` are, hashing only where some slot lists an image
/// of that length.
fn identify<'a>(
    slots: &'a [FirmwareSlot],
    bytes: &[u8],
) -> Option<(&'static str, &'a FirmwareImage)> {
    if !slots
        .iter()
        .any(|slot| slot.images.iter().any(|image| image.size == bytes.len()))
    {
        return None;
    }
    let hash = sha256_hex(bytes);
    slots.iter().find_map(|slot| {
        slot.images
            .iter()
            .find(|image| image.size == bytes.len() && image.sha256 == hash)
            .map(|image| (slot.id, image))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::firmware::FirmwareOrigin;

    /// A slot over synthetic images, hashed from the bytes the tests write.
    fn slot(need: FirmwareNeed) -> FirmwareSlot {
        FirmwareSlot {
            id: "boot-rom",
            label: "Boot ROM",
            need,
            images: vec![
                FirmwareImage {
                    id: "first",
                    label: "First",
                    size: 4,
                    sha256: leaked(&first()),
                    origin: FirmwareOrigin::Official,
                },
                FirmwareImage {
                    id: "second",
                    label: "Second",
                    size: 4,
                    sha256: leaked(&second()),
                    origin: FirmwareOrigin::Open { project: "test" },
                },
            ],
        }
    }

    fn first() -> Vec<u8> {
        vec![0x31, 0xFE, 0xFF, 0xAF]
    }

    fn second() -> Vec<u8> {
        vec![0x00, 0x01, 0x02, 0x03]
    }

    /// The synthetic hashes are computed, not pasted, so no dump is needed.
    fn leaked(bytes: &[u8]) -> &'static str {
        sha256_hex(bytes).leak()
    }

    fn descriptors(slot: FirmwareSlot) -> Vec<LaunchOptionDescriptor> {
        vec![LaunchOptionDescriptor {
            id: slot.id,
            label: slot.label,
            kind: LaunchOptionKind::Firmware { slot },
        }]
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("missingno-firmware-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary firmware folder");
        dir
    }

    #[test]
    fn a_file_is_recognised_by_content_whatever_it_is_called() {
        let dir = temp_dir("by-content");
        std::fs::write(dir.join("anything.dat"), first()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not an image").unwrap();

        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        let present = library.present(&slot);
        assert_eq!(present.len(), 1);
        assert_eq!(present[0].image.id, "first");
        assert_eq!(present[0].path, dir.join("anything.dat"));
        assert_eq!(library.unrecognised(), [dir.join("notes.txt")]);
    }

    #[test]
    fn one_image_in_two_files_is_present_once() {
        let dir = temp_dir("duplicate");
        std::fs::write(dir.join("a.bin"), first()).unwrap();
        std::fs::write(dir.join("b.bin"), first()).unwrap();

        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        assert_eq!(library.present(&slot).len(), 1);
        assert_eq!(library.present(&slot)[0].path, dir.join("a.bin"));
        assert!(library.unrecognised().is_empty());
    }

    #[test]
    fn an_empty_folder_is_scanned_into_existence() {
        let dir = temp_dir("created");
        std::fs::remove_dir_all(&dir).unwrap();
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        assert!(dir.is_dir());
        assert!(library.present(&slot).is_empty());
    }

    #[test]
    fn the_automatic_pick_follows_the_need() {
        let dir = temp_dir("automatic");
        std::fs::write(dir.join("second.bin"), second()).unwrap();

        let optional = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&optional));
        assert!(library.automatic(&optional, None).is_none());
        assert_eq!(
            library.automatic(&optional, Some("second")).map(id),
            Some("second")
        );
        // A default whose file was removed resolves to nothing.
        assert!(library.automatic(&optional, Some("first")).is_none());

        let required = slot(FirmwareNeed::Required);
        assert_eq!(library.automatic(&required, None).map(id), Some("second"));
    }

    fn id(present: &PresentImage) -> &'static str {
        present.image.id
    }

    #[test]
    fn a_chosen_image_becomes_the_bytes_the_core_reads() {
        let dir = temp_dir("supply");
        std::fs::write(dir.join("image.bin"), first()).unwrap();
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir, std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        values.set_choice("boot-rom", "first");
        library
            .supply(&published, &mut values)
            .expect("it is there");
        assert_eq!(values.file("boot-rom"), Some(first().as_slice()));
    }

    #[test]
    fn an_optional_socket_left_alone_or_emptied_stays_empty() {
        let dir = temp_dir("optional");
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir, std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        library.supply(&published, &mut values).expect("automatic");
        assert!(values.is_empty());

        values.set_choice("boot-rom", FIRMWARE_NONE);
        library.supply(&published, &mut values).expect("none");
        assert!(values.is_empty());
    }

    #[test]
    fn a_required_socket_refuses_an_empty_folder() {
        let dir = temp_dir("required");
        let slot = slot(FirmwareNeed::Required);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        assert_eq!(
            library.supply(&published, &mut values),
            Err(FirmwareRefusal::Missing {
                slot: "boot-rom",
                label: "Boot ROM",
                dir: dir.clone(),
            })
        );

        values.set_choice("boot-rom", FIRMWARE_NONE);
        assert_eq!(
            library.supply(&published, &mut values),
            Err(FirmwareRefusal::NoneNotAllowed { slot: "boot-rom" })
        );
    }

    #[test]
    fn an_image_the_folder_lacks_and_an_id_the_slot_lacks_both_refuse() {
        let dir = temp_dir("refusals");
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        values.set_choice("boot-rom", "first");
        assert_eq!(
            library.supply(&published, &mut values),
            Err(FirmwareRefusal::NotInFolder {
                slot: "boot-rom",
                image: "first".to_owned(),
                dir,
            })
        );

        values.set_choice("boot-rom", "agb");
        assert_eq!(
            library.supply(&published, &mut values),
            Err(FirmwareRefusal::UnknownImage {
                slot: "boot-rom",
                image: "agb".to_owned(),
            })
        );
    }

    #[test]
    fn a_file_supplied_outright_is_left_alone() {
        let dir = temp_dir("outright");
        let slot = slot(FirmwareNeed::Required);
        let library = FirmwareLibrary::scan(dir, std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        values.set_file("boot-rom", vec![0xAA; 4]);
        library
            .supply(&published, &mut values)
            .expect("the caller's own image");
        assert_eq!(values.file("boot-rom"), Some([0xAA; 4].as_slice()));
    }
}
