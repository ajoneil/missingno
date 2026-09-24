//! The firmware folder: the files a user dropped in it, matched against the
//! images the loaded cores recognise.
//!
//! A core states its sockets ([`FirmwareSlot`]) and a caller states which image
//! to fill one with by id; this is where an id becomes bytes. Identity is
//! content, so a file's name is only what it is displayed as — a dump saved as
//! `boot.bin` is recognised as readily as one saved under its model's name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use missingno_core::firmware::{
    FirmwareImage, FirmwareNeed, FirmwareSlot, FirmwareValue, firmware_slots,
};
use missingno_core::launch::{LaunchOptionDescriptor, LaunchValues, TV_STANDARD};
use missingno_core::tv::TvStandard;
use serde::{Deserialize, Serialize};

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

/// The image a socket takes when nobody chooses, as a frontend persists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FirmwareDefault {
    /// One answer, for a socket whose images are not cut by standard.
    Image(String),
    /// One answer per standard, for a socket whose images are.
    ByStandard(BTreeMap<TvStandard, String>),
}

/// Every socket's default, keyed by socket id. Sparse: an absent socket is
/// left to the automatic pick.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FirmwareDefaults(BTreeMap<String, FirmwareDefault>);

impl FirmwareDefaults {
    /// The image a launch of `standard` takes in `slot` when nobody chooses.
    pub fn image_for(&self, slot: &str, standard: Option<TvStandard>) -> Option<&str> {
        match self.0.get(slot)? {
            FirmwareDefault::Image(image) => Some(image),
            FirmwareDefault::ByStandard(images) => images.get(&standard?).map(String::as_str),
        }
    }

    /// State `slot`'s default for `standard`, or for every launch when no
    /// standard is given; `None` leaves it to the automatic pick.
    pub fn set(&mut self, slot: &str, standard: Option<TvStandard>, image: Option<String>) {
        match (standard, image) {
            (None, Some(image)) => {
                self.0
                    .insert(slot.to_owned(), FirmwareDefault::Image(image));
            }
            (None, None) => {
                self.0.remove(slot);
            }
            (Some(standard), Some(image)) => {
                let entry = self
                    .0
                    .entry(slot.to_owned())
                    .or_insert_with(|| FirmwareDefault::ByStandard(BTreeMap::new()));
                if let FirmwareDefault::Image(_) = entry {
                    *entry = FirmwareDefault::ByStandard(BTreeMap::new());
                }
                if let FirmwareDefault::ByStandard(images) = entry {
                    images.insert(standard, image);
                }
            }
            (Some(standard), None) => {
                if let Some(FirmwareDefault::ByStandard(images)) = self.0.get_mut(slot) {
                    images.remove(&standard);
                    if images.is_empty() {
                        self.0.remove(slot);
                    }
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why a launch cannot be given the firmware it asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirmwareRefusal {
    /// A socket that must be filled, with nothing filling it.
    Required {
        slot: &'static str,
        label: &'static str,
        dir: PathBuf,
    },
    /// A recognised image, but no file in the folder holds it.
    NotInFolder {
        label: &'static str,
        image: &'static str,
        dir: PathBuf,
    },
    /// An image id the slot does not list.
    UnknownImage { label: &'static str, image: String },
}

impl std::fmt::Display for FirmwareRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FirmwareRefusal::Required { label, dir, .. } => {
                write!(f, "this console needs a {label} in {}", dir.display())
            }
            FirmwareRefusal::NotInFolder { label, image, dir } => write!(
                f,
                "no file in {} holds the {label} \"{image}\"",
                dir.display()
            ),
            FirmwareRefusal::UnknownImage { label, image } => {
                write!(f, "{label}: no image \"{image}\"")
            }
        }
    }
}

impl FirmwareLibrary {
    /// Where firmware lives when a caller names no folder of its own.
    pub fn default_dir() -> Option<PathBuf> {
        crate::config_dir().map(|dir| dir.join("firmware"))
    }

    /// The default folder, read against every socket this build's cores state.
    pub fn scan_default() -> Self {
        Self::scan(
            Self::default_dir().unwrap_or_default(),
            &crate::factory::firmware_slots(),
        )
    }

    /// Read `dir`, naming every file that holds an image one of `slots` lists.
    /// A folder that does not exist yet simply holds nothing.
    pub fn scan(dir: PathBuf, slots: &[FirmwareSlot]) -> Self {
        let sizes: Vec<usize> = slots.iter().map(|slot| slot.size).collect();
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
            let found = read_candidate(&path, &sizes).and_then(|bytes| identify(slots, &bytes));
            match found {
                Some((slot, image)) => {
                    let already = present
                        .iter()
                        .any(|found| found.slot == slot && found.image.id == image.id);
                    if !already {
                        present.push(PresentImage { slot, image, path });
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

    /// Make the folder, for a frontend about to show it to the user.
    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
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

    /// What a slot resolves to when nobody chose: the stated default if the
    /// folder holds it, else — for a socket that must be filled — the first
    /// present image cut for `standard`, else the first present image, else
    /// nothing.
    pub fn automatic(
        &self,
        slot: &FirmwareSlot,
        default: Option<&str>,
        standard: Option<TvStandard>,
    ) -> Option<&PresentImage> {
        if let Some(found) = default.and_then(|id| self.find(slot.id, id)) {
            return Some(found);
        }
        match slot.need {
            FirmwareNeed::Required => {
                let present = self.present(slot);
                let cut_for_standard = standard.and_then(|standard| {
                    present
                        .iter()
                        .find(|found| found.image.standard == Some(standard))
                        .copied()
                });
                cut_for_standard.or_else(|| present.first().copied())
            }
            FirmwareNeed::Optional => None,
        }
    }

    /// Turn every firmware choice in `values` into the bytes a core reads,
    /// filling a socket nobody chose for from `defaults` and the automatic
    /// pick. A caller that supplied the bytes outright keeps them, and a socket
    /// the machine starts without may be left empty.
    pub fn supply(
        &self,
        descriptors: &[LaunchOptionDescriptor],
        values: &mut LaunchValues,
        defaults: &FirmwareDefaults,
    ) -> Result<(), FirmwareRefusal> {
        let standard = values.choice(TV_STANDARD).and_then(TvStandard::from_name);
        for slot in firmware_slots(descriptors) {
            let required = slot.need == FirmwareNeed::Required;
            let empty = || match required {
                true => Err(FirmwareRefusal::Required {
                    slot: slot.id,
                    label: slot.label,
                    dir: self.dir.clone(),
                }),
                false => Ok(()),
            };
            match values.firmware(slot.id) {
                None => {
                    let default = defaults.image_for(slot.id, standard);
                    match self.automatic(slot, default, standard) {
                        Some(found) => {
                            let bytes = self.image_bytes(slot, found.image.id)?;
                            values.set_firmware(slot.id, FirmwareValue::Bytes(bytes));
                        }
                        None => empty()?,
                    }
                }
                Some(FirmwareValue::None) => empty()?,
                Some(FirmwareValue::Bytes(_)) => {}
                Some(FirmwareValue::Image(chosen)) => {
                    let chosen = chosen.clone();
                    let bytes = self.image_bytes(slot, &chosen)?;
                    values.set_firmware(slot.id, FirmwareValue::Bytes(bytes));
                }
            }
        }
        Ok(())
    }

    fn image_bytes(&self, slot: &FirmwareSlot, image: &str) -> Result<Vec<u8>, FirmwareRefusal> {
        let Some(listed) = slot.image(image) else {
            return Err(FirmwareRefusal::UnknownImage {
                label: slot.label,
                image: image.to_owned(),
            });
        };
        let not_in_folder = || FirmwareRefusal::NotInFolder {
            label: slot.label,
            image: listed.label,
            dir: self.dir.clone(),
        };
        let found = self.find(slot.id, image).ok_or_else(not_in_folder)?;
        std::fs::read(&found.path).map_err(|_| not_in_folder())
    }
}

/// A file's contents, unless its length is one no socket takes.
fn read_candidate(path: &Path, sizes: &[usize]) -> Option<Vec<u8>> {
    let length = std::fs::metadata(path).ok()?.len();
    sizes
        .iter()
        .any(|size| *size as u64 == length)
        .then(|| std::fs::read(path).ok())
        .flatten()
}

/// The slot and image `bytes` are, if any slot lists one with that content.
fn identify(slots: &[FirmwareSlot], bytes: &[u8]) -> Option<(&'static str, FirmwareImage)> {
    slots
        .iter()
        .find_map(|slot| slot.identify(bytes).map(|image| (slot.id, *image)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::launch::LaunchOptionKind;

    /// A slot over synthetic images, hashed from the bytes the tests write.
    fn slot(need: FirmwareNeed) -> FirmwareSlot {
        FirmwareSlot {
            id: "boot-rom",
            label: "Boot ROM",
            need,
            size: 4,
            images: images(),
        }
    }

    /// The synthetic hashes are computed, not pasted, so no dump is needed.
    fn images() -> &'static [FirmwareImage] {
        Box::leak(Box::new([
            FirmwareImage::official("first", "First", leaked(&first())),
            FirmwareImage::open("second", "Second", "test", leaked(&second())),
        ]))
    }

    fn first() -> Vec<u8> {
        vec![0x31, 0xFE, 0xFF, 0xAF]
    }

    fn second() -> Vec<u8> {
        vec![0x00, 0x01, 0x02, 0x03]
    }

    fn leaked(bytes: &[u8]) -> &'static str {
        missingno_core::firmware::sha256_hex(bytes).leak()
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
    fn a_folder_that_is_not_there_holds_nothing_and_is_not_made() {
        let dir = temp_dir("absent");
        std::fs::remove_dir_all(&dir).unwrap();
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        assert!(!dir.exists());
        assert!(library.present(&slot).is_empty());

        library.ensure_dir().expect("the folder is made on request");
        assert!(dir.is_dir());
    }

    /// A socket over one image cut for NTSC, one for PAL and one cut for
    /// neither, in that order.
    fn cut_slot(need: FirmwareNeed) -> FirmwareSlot {
        FirmwareSlot {
            id: "bios",
            label: "BIOS",
            need,
            size: 4,
            images: Box::leak(Box::new([
                FirmwareImage::official("ntsc", "Console NTSC", leaked(&ntsc()))
                    .cut_for(TvStandard::Ntsc),
                FirmwareImage::official("pal", "Console PAL", leaked(&pal()))
                    .cut_for(TvStandard::Pal),
                FirmwareImage::official("uncut", "Console", leaked(&uncut())),
            ])),
        }
    }

    fn ntsc() -> Vec<u8> {
        vec![0x4E, 0x54, 0x53, 0x43]
    }

    fn pal() -> Vec<u8> {
        vec![0x50, 0x41, 0x4C, 0x00]
    }

    fn uncut() -> Vec<u8> {
        vec![0x55, 0x4E, 0x43, 0x54]
    }

    /// A folder holding `files`, read against `slot`.
    fn library_holding(name: &str, slot: &FirmwareSlot, files: &[Vec<u8>]) -> FirmwareLibrary {
        let dir = temp_dir(name);
        for (index, bytes) in files.iter().enumerate() {
            std::fs::write(dir.join(format!("{index}.bin")), bytes).unwrap();
        }
        FirmwareLibrary::scan(dir, std::slice::from_ref(slot))
    }

    #[test]
    fn the_automatic_pick_follows_the_need() {
        let dir = temp_dir("automatic");
        std::fs::write(dir.join("second.bin"), second()).unwrap();

        let optional = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&optional));
        assert!(library.automatic(&optional, None, None).is_none());
        assert_eq!(
            library.automatic(&optional, Some("second"), None).map(id),
            Some("second")
        );
        // A default whose file was removed resolves to nothing.
        assert!(library.automatic(&optional, Some("first"), None).is_none());

        let required = slot(FirmwareNeed::Required);
        assert_eq!(
            library.automatic(&required, None, None).map(id),
            Some("second")
        );
    }

    #[test]
    fn the_automatic_pick_prefers_the_default_then_the_standard_then_any_image() {
        let required = cut_slot(FirmwareNeed::Required);
        let all = library_holding("cut-all", &required, &[uncut(), pal(), ntsc()]);
        let pick = |library: &FirmwareLibrary, default, standard| {
            library.automatic(&required, default, standard).map(id)
        };

        assert_eq!(pick(&all, Some("pal"), Some(TvStandard::Ntsc)), Some("pal"));
        assert_eq!(pick(&all, None, Some(TvStandard::Pal)), Some("pal"));
        assert_eq!(pick(&all, None, Some(TvStandard::Ntsc)), Some("ntsc"));
        assert_eq!(pick(&all, None, None), Some("ntsc"));
        assert_eq!(pick(&all, None, Some(TvStandard::Secam)), Some("ntsc"));

        let uncut_only = library_holding("cut-uncut", &required, &[uncut()]);
        assert_eq!(
            pick(&uncut_only, Some("pal"), Some(TvStandard::Pal)),
            Some("uncut")
        );

        let optional = cut_slot(FirmwareNeed::Optional);
        let library = library_holding("cut-optional", &optional, &[pal()]);
        assert!(
            library
                .automatic(&optional, None, Some(TvStandard::Pal))
                .is_none()
        );
    }

    /// Supply a launch of `standard` over `library`, returning the bytes the
    /// slot resolved to.
    fn supplied(
        library: &FirmwareLibrary,
        slot: FirmwareSlot,
        standard: Option<TvStandard>,
        defaults: &FirmwareDefaults,
    ) -> Result<Option<FirmwareValue>, FirmwareRefusal> {
        let id = slot.id;
        let mut values = LaunchValues::default();
        if let Some(standard) = standard {
            values.set_choice(TV_STANDARD, standard.name());
        }
        library.supply(&descriptors(slot), &mut values, defaults)?;
        Ok(values.firmware(id).cloned())
    }

    #[test]
    fn a_lone_image_fills_a_required_socket_whatever_the_standard() {
        let slot = cut_slot(FirmwareNeed::Required);
        let library = library_holding("supply-lone-pal", &slot, &[pal()]);
        assert_eq!(
            supplied(&library, slot, None, &FirmwareDefaults::default()),
            Ok(Some(FirmwareValue::Bytes(pal())))
        );
    }

    #[test]
    fn with_both_images_the_launch_standard_picks() {
        let slot = cut_slot(FirmwareNeed::Required);
        let library = library_holding("supply-both", &slot, &[pal(), ntsc()]);
        let defaults = FirmwareDefaults::default();
        assert_eq!(
            supplied(&library, slot, Some(TvStandard::Pal), &defaults),
            Ok(Some(FirmwareValue::Bytes(pal())))
        );
        assert_eq!(
            supplied(&library, slot, Some(TvStandard::Ntsc), &defaults),
            Ok(Some(FirmwareValue::Bytes(ntsc())))
        );
        assert_eq!(
            supplied(&library, slot, None, &defaults),
            Ok(Some(FirmwareValue::Bytes(ntsc())))
        );
    }

    #[test]
    fn a_pal_launch_takes_the_ntsc_image_when_it_is_all_there_is() {
        let slot = cut_slot(FirmwareNeed::Required);
        let library = library_holding("supply-lone-ntsc", &slot, &[ntsc()]);
        assert_eq!(
            supplied(
                &library,
                slot,
                Some(TvStandard::Pal),
                &FirmwareDefaults::default()
            ),
            Ok(Some(FirmwareValue::Bytes(ntsc())))
        );
    }

    #[test]
    fn a_default_for_the_standard_outranks_the_standards_own_image() {
        let slot = cut_slot(FirmwareNeed::Required);
        let library = library_holding("supply-default", &slot, &[pal(), ntsc()]);
        let mut defaults = FirmwareDefaults::default();
        defaults.set("bios", Some(TvStandard::Pal), Some("ntsc".to_owned()));
        assert_eq!(
            supplied(&library, slot, Some(TvStandard::Pal), &defaults),
            Ok(Some(FirmwareValue::Bytes(ntsc())))
        );
    }

    #[test]
    fn an_optional_socket_with_no_default_stays_empty_beside_present_images() {
        let slot = slot(FirmwareNeed::Optional);
        let library = library_holding("supply-optional", &slot, &[first(), second()]);
        assert_eq!(
            supplied(&library, slot, None, &FirmwareDefaults::default()),
            Ok(None)
        );
    }

    #[test]
    fn a_refused_image_is_named_by_its_labels() {
        let dir = PathBuf::from("/firmware");
        let refusal = FirmwareRefusal::NotInFolder {
            label: "BIOS",
            image: "ColecoVision PAL (1983)",
            dir,
        };
        assert_eq!(
            refusal.to_string(),
            "no file in /firmware holds the BIOS \"ColecoVision PAL (1983)\""
        );
        let unknown = FirmwareRefusal::UnknownImage {
            label: "BIOS",
            image: "secam".to_owned(),
        };
        assert_eq!(unknown.to_string(), "BIOS: no image \"secam\"");
    }

    #[test]
    fn a_missing_override_is_refused_with_the_images_label() {
        let slot = cut_slot(FirmwareNeed::Required);
        let library = library_holding("supply-missing", &slot, &[ntsc()]);
        let mut values = LaunchValues::default();
        values.set_firmware("bios", FirmwareValue::Image("pal".to_owned()));
        let refusal = library
            .supply(
                &descriptors(slot),
                &mut values,
                &FirmwareDefaults::default(),
            )
            .unwrap_err();
        assert_eq!(
            refusal.to_string(),
            format!(
                "no file in {} holds the BIOS \"Console PAL\"",
                library.dir().display()
            )
        );
    }

    #[test]
    fn defaults_answer_per_shape() {
        let mut defaults = FirmwareDefaults::default();
        assert!(defaults.is_empty());
        defaults.set("dmg-boot-rom", None, Some("dmg0".to_owned()));
        defaults.set("bios", Some(TvStandard::Pal), Some("ntsc".to_owned()));
        assert_eq!(defaults.image_for("dmg-boot-rom", None), Some("dmg0"));
        assert_eq!(
            defaults.image_for("dmg-boot-rom", Some(TvStandard::Pal)),
            Some("dmg0")
        );
        assert_eq!(
            defaults.image_for("bios", Some(TvStandard::Pal)),
            Some("ntsc")
        );
        assert_eq!(defaults.image_for("bios", Some(TvStandard::Ntsc)), None);
        assert_eq!(defaults.image_for("bios", None), None);

        defaults.set("bios", Some(TvStandard::Pal), None);
        defaults.set("dmg-boot-rom", None, None);
        assert!(defaults.is_empty());
    }

    #[test]
    fn defaults_round_trip_both_shapes_through_ron() {
        let mut defaults = FirmwareDefaults::default();
        defaults.set("dmg-boot-rom", None, Some("dmg0".to_owned()));
        defaults.set("bios", Some(TvStandard::Ntsc), Some("ntsc".to_owned()));
        defaults.set("bios", Some(TvStandard::Pal), Some("pal".to_owned()));
        let text = ron::to_string(&defaults).expect("serialises");
        let read: FirmwareDefaults = ron::from_str(&text).expect("deserialises");
        assert_eq!(read, defaults);

        let legacy: FirmwareDefaults =
            ron::from_str(r#"{"dmg-boot-rom": "dmg0"}"#).expect("the old shape");
        assert_eq!(legacy.image_for("dmg-boot-rom", None), Some("dmg0"));
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
        values.set_firmware("boot-rom", FirmwareValue::Image("first".to_owned()));
        library
            .supply(&published, &mut values, &FirmwareDefaults::default())
            .expect("it is there");
        assert_eq!(
            values.firmware("boot-rom"),
            Some(&FirmwareValue::Bytes(first()))
        );
    }

    #[test]
    fn an_optional_socket_left_alone_or_emptied_stays_empty() {
        let dir = temp_dir("optional");
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir, std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        library
            .supply(&published, &mut values, &FirmwareDefaults::default())
            .expect("automatic");
        assert!(values.is_empty());

        values.set_firmware("boot-rom", FirmwareValue::None);
        library
            .supply(&published, &mut values, &FirmwareDefaults::default())
            .expect("none");
        assert_eq!(values.firmware("boot-rom"), Some(&FirmwareValue::None));
    }

    #[test]
    fn a_required_socket_refuses_an_empty_folder() {
        let dir = temp_dir("required");
        let slot = slot(FirmwareNeed::Required);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        let published = descriptors(slot);
        let refusal = FirmwareRefusal::Required {
            slot: "boot-rom",
            label: "Boot ROM",
            dir,
        };

        let mut values = LaunchValues::default();
        assert_eq!(
            library.supply(&published, &mut values, &FirmwareDefaults::default()),
            Err(refusal.clone())
        );

        values.set_firmware("boot-rom", FirmwareValue::None);
        assert_eq!(
            library.supply(&published, &mut values, &FirmwareDefaults::default()),
            Err(refusal)
        );
    }

    #[test]
    fn an_image_the_folder_lacks_and_an_id_the_slot_lacks_both_refuse() {
        let dir = temp_dir("refusals");
        let slot = slot(FirmwareNeed::Optional);
        let library = FirmwareLibrary::scan(dir.clone(), std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        values.set_firmware("boot-rom", FirmwareValue::Image("first".to_owned()));
        assert_eq!(
            library.supply(&published, &mut values, &FirmwareDefaults::default()),
            Err(FirmwareRefusal::NotInFolder {
                label: "Boot ROM",
                image: "First",
                dir,
            })
        );

        values.set_firmware("boot-rom", FirmwareValue::Image("agb".to_owned()));
        assert_eq!(
            library.supply(&published, &mut values, &FirmwareDefaults::default()),
            Err(FirmwareRefusal::UnknownImage {
                label: "Boot ROM",
                image: "agb".to_owned(),
            })
        );
    }

    #[test]
    fn bytes_supplied_outright_are_left_alone() {
        let dir = temp_dir("outright");
        let slot = slot(FirmwareNeed::Required);
        let library = FirmwareLibrary::scan(dir, std::slice::from_ref(&slot));
        let published = descriptors(slot);

        let mut values = LaunchValues::default();
        values.set_firmware("boot-rom", FirmwareValue::Bytes(vec![0xAA; 4]));
        library
            .supply(&published, &mut values, &FirmwareDefaults::default())
            .expect("the caller's own image");
        assert_eq!(
            values.firmware("boot-rom"),
            Some(&FirmwareValue::Bytes(vec![0xAA; 4]))
        );
    }
}
