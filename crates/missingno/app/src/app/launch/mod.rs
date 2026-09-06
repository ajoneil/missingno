//! What the user decides about a ROM before it boots, in one place.
//!
//! Every row the launch window shows comes from the family's own launch
//! descriptors, so a core that publishes a new option gets a row without a
//! change here. An option the user has not set is absent — absence is what
//! leaves the decision to the core — and what the catalogue states about a
//! dump is read live at launch rather than copied onto the library entry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use iced::Task;
use missingno_core::cartridge::BoardValue;
use missingno_core::firmware::{FirmwareValue, firmware_slots};
use missingno_core::launch::{LaunchOptionDescriptor, LaunchValue, LaunchValues};
use missingno_gamedb::{Enhancement, Peripheral};
use missingno_session::FirmwareLibrary;

use crate::app::library::catalogue::{Catalogue, CatalogueRelease};
use crate::app::system::{self, FamilyDescriptor, Platform};
use crate::app::{self, App, library, load};

mod view;

pub(in crate::app) use view::{PanelData, panel, window};

/// What fills the options the user has not set, keyed by option id. Sparse: an
/// option nothing here names is one only the core can resolve.
#[derive(Clone, Default)]
pub struct Facts(BTreeMap<&'static str, LaunchValue>);

impl Facts {
    fn set(&mut self, id: &'static str, value: LaunchValue) {
        self.0.insert(id, value);
    }

    pub fn get(&self, id: &str) -> Option<&LaunchValue> {
        self.0.get(id)
    }
}

/// Options no launch surface renders: an overdump is the catalogue's word about
/// a dump rather than a choice. It still resolves into the values a launch runs
/// with.
const UNRENDERED_OPTIONS: [&str; 1] = [system::vcs::OVERDUMP];

fn rendered(descriptor: &LaunchOptionDescriptor) -> bool {
    !UNRENDERED_OPTIONS.contains(&descriptor.id)
}

/// Everything that fills a launch option besides the user: the game database,
/// the firmware folder, the defaults the settings keep for its sockets, and the
/// image this run was started with.
pub struct LaunchSources<'a> {
    pub catalogue: &'a Catalogue,
    pub firmware: &'a FirmwareLibrary,
    /// The image each socket takes when nobody chooses, keyed by socket id.
    pub defaults: &'a BTreeMap<String, String>,
    /// The socket the command line's own image fills, and the image.
    pub cli_firmware: Option<&'a (&'static str, FirmwareValue)>,
}

/// The options this media publishes under these values and what fills the ones
/// the user left alone, read against one publication: the rows follow the
/// values, so the facts must answer the very rows that will be shown.
pub fn plan(
    family: &FamilyDescriptor,
    rom: &[u8],
    overrides: &LaunchValues,
    sha1: &str,
    sources: &LaunchSources<'_>,
) -> (Vec<LaunchOptionDescriptor>, Facts) {
    let descriptors = (family.options)(rom, overrides);
    let facts = facts(family, rom, &descriptors, sha1, sources);
    (descriptors, facts)
}

/// What the catalogue and the media itself state about a dump, ahead of any
/// word from the user: the game database's facts about this hash, the header's
/// own, the firmware folder's answer to each socket, and the image this run was
/// started with.
fn facts(
    family: &FamilyDescriptor,
    rom: &[u8],
    descriptors: &[LaunchOptionDescriptor],
    sha1: &str,
    sources: &LaunchSources<'_>,
) -> Facts {
    let mut facts = Facts::default();

    for stated in (family.stated_by_media)(rom) {
        facts.set(stated.option, stated.value);
    }

    if let Some((_, release, artifact)) = sources.catalogue.lookup_hash(sha1) {
        stated_by_release(&mut facts, release);
        // A dump padded past the cartridge's silicon: the stated board says
        // where the silicon ends.
        facts.set(
            system::vcs::OVERDUMP,
            LaunchValue::Toggle(artifact.defect == Some(missingno_gamedb::Defect::Overdump)),
        );
    }

    for slot in firmware_slots(descriptors) {
        let chosen = sources.defaults.get(slot.id).map(String::as_str);
        if let Some(present) = sources.firmware.automatic(slot, chosen) {
            facts.set(
                slot.id,
                LaunchValue::Firmware(FirmwareValue::Image(present.image.id.to_owned())),
            );
        }
    }

    // The image the run was started with outranks the folder's own answer.
    if let Some((slot, image)) = sources.cli_firmware {
        facts.set(slot, LaunchValue::Firmware(image.clone()));
    }

    facts
}

/// What the catalogue's word on a release fills, over the media's own.
fn stated_by_release(facts: &mut Facts, release: &CatalogueRelease) {
    if let Some(standard) = release.tv_format {
        facts.set(
            system::vcs::TV_STANDARD,
            LaunchValue::Choice(standard.name().to_owned()),
        );
    }
    // Every family publishes its board option under the same id, so one key
    // carries the catalogue's word whichever core is about to read it.
    if let Some(board) = &release.cart_type {
        facts.set(system::vcs::BOARD, LaunchValue::Board(board.clone()));
    }
    // A stated enhancement list is the whole word on what the release exploits,
    // header included, so it replaces what the header claimed.
    if let Some(enhancements) = &release.enhancements {
        facts.set(
            system::gb::ENHANCEMENTS,
            LaunchValue::Flags(stated_enhancements(enhancements)),
        );
    }
    if let Some(runner) = stated_runner(release) {
        facts.set(system::gb::RUNNER, LaunchValue::Choice(runner.to_owned()));
    }
}

/// The Game Boy family's flag ids for the enhancements a release states.
fn stated_enhancements(enhancements: &[Enhancement]) -> BTreeSet<String> {
    enhancements
        .iter()
        .map(|enhancement| {
            match enhancement {
                Enhancement::SuperGameBoy => system::gb::ENHANCEMENT_SGB,
                Enhancement::GameBoyColor => system::gb::ENHANCEMENT_CGB,
            }
            .to_owned()
        })
        .collect()
}

/// The console a Game Boy release's stated enhancements name: one whose list
/// leaves the Color out plays on a Game Boy however its header is flagged. A
/// release stating no enhancements leaves the media to answer.
fn stated_runner(release: &CatalogueRelease) -> Option<&'static str> {
    release.enhancements.as_ref().map(|enhancements| {
        match enhancements.contains(&Enhancement::GameBoyColor) {
            true => "cgb",
            false => "dmg",
        }
    })
}

/// The peripherals the catalogue says this dump's release is played with; empty
/// leaves the console's power-on configuration.
pub fn catalogued_peripherals(catalogue: &Catalogue, sha1: &str) -> Vec<Peripheral> {
    catalogue
        .lookup_hash(sha1)
        .map(|(_, release, _)| release.peripherals.clone())
        .unwrap_or_default()
}

/// The values a launch runs with: the user's own word on an option wins, else
/// whatever fact fills it, else nothing — which leaves the option to the core.
/// An override for an option these descriptors do not name is left aside rather
/// than lost: the choices that publish it may come back.
pub fn resolve(
    descriptors: &[LaunchOptionDescriptor],
    overrides: &LaunchValues,
    facts: &Facts,
) -> LaunchValues {
    let mut values = LaunchValues::default();
    for descriptor in descriptors {
        if let Some(value) = overrides
            .value(descriptor.id)
            .or_else(|| facts.get(descriptor.id))
        {
            values.set(descriptor.id, value.clone());
        }
    }
    // Enhancements name the console themselves, so a user who states them
    // without naming one leaves the header's word out of the answer.
    if overrides.flags(system::gb::ENHANCEMENTS).is_some()
        && overrides.value(system::gb::RUNNER).is_none()
    {
        values.clear(system::gb::RUNNER);
    }
    values
}

/// What the window launches, and where the user's edits are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A library game: every edit is written to its entry as it is made.
    Library,
    /// An arbitrary ROM: the values apply to this launch alone.
    Transient,
}

/// Which surface an edit was made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSurface {
    /// The launch window standing over the screen.
    Window,
    /// A library game's own settings section on its details page.
    GameSettings,
}

/// One change to an option's value. `None` is Automatic: the user's word is
/// dropped, and whatever fills the option fills it again.
#[derive(Debug, Clone)]
pub enum Edit {
    Choice(&'static str, Option<String>),
    /// The whole set of flags that are on, as one edit.
    Flags(&'static str, Option<BTreeSet<String>>),
    Toggle(&'static str, Option<bool>),
    /// What fills a firmware socket; `None` hands it back to the folder.
    Firmware(&'static str, Option<FirmwareValue>),
    /// A whole board and the parts stated on it, as one edit.
    Board(&'static str, Option<BoardValue>),
}

impl Edit {
    fn apply(&self, values: &mut LaunchValues) {
        match self {
            Edit::Choice(id, Some(value)) => values.set_choice(*id, value.clone()),
            Edit::Flags(id, Some(flags)) => values.set_flags(*id, flags.clone()),
            Edit::Toggle(id, Some(value)) => values.set_toggle(*id, *value),
            Edit::Firmware(id, Some(image)) => values.set_firmware(*id, image.clone()),
            Edit::Board(id, Some(board)) => values.set_board(*id, board.clone()),
            Edit::Choice(id, None)
            | Edit::Flags(id, None)
            | Edit::Toggle(id, None)
            | Edit::Firmware(id, None)
            | Edit::Board(id, None) => values.clear(id),
        }
    }
}

/// The launch window: one ROM, the options its family publishes, and what the
/// user has decided about them.
pub struct Window {
    pub rom_path: PathBuf,
    pub rom: Vec<u8>,
    pub sha1: String,
    /// What the window calls what it is about to boot.
    title: String,
    /// Whose options the rows render. `None` while media no family claims waits
    /// for the user to name the system it is for.
    pub platform: Option<Platform>,
    /// Whether anything identified the media — a family's predicate or the
    /// catalogue's word on its hash. What nothing does asks the user first.
    claimed: bool,
    /// The user's own word on the options, as this window has it.
    pub overrides: LaunchValues,
    /// The rows these values publish, and what fills the ones nobody set. Both
    /// move with an edit, so both are read again together.
    rows: Vec<LaunchOptionDescriptor>,
    facts: Facts,
    pub target: Target,
    /// What the last attempt to launch was refused with.
    pub error: Option<String>,
}

impl Window {
    pub fn family(&self) -> Option<&'static FamilyDescriptor> {
        self.platform.and_then(system::family_of)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Open the window for a library game, where edits persist to its entry.
    OpenForGame(String),
    /// Open the window for the ROM at a path, for this launch alone.
    OpenForPath(PathBuf),
    /// Open the window over media in hand.
    Opened(PathBuf, Vec<u8>, Target),
    /// Name the system media no family claimed is for.
    SelectSystem(Platform),
    Set(EditSurface, Edit),
    Launch,
    Close,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Launch(message)
    }
}

pub fn update(message: Message, app: &mut App) -> Task<app::Message> {
    match message {
        Message::OpenForGame(sha1) => {
            let Some((_, entry)) = library::find_by_sha1(&sha1) else {
                return Task::none();
            };
            let Some(rom_path) = entry.rom_paths.iter().find(|path| path.exists()).cloned() else {
                return Task::none();
            };
            let Ok(rom) = std::fs::read(&rom_path) else {
                return Task::none();
            };
            let rom = crate::patch::soft_patch(&rom_path, rom);
            return Task::done(Message::Opened(rom_path, rom, Target::Library).into());
        }

        Message::OpenForPath(rom_path) => {
            let Ok(rom) = std::fs::read(&rom_path) else {
                return Task::none();
            };
            let rom = crate::patch::soft_patch(&rom_path, rom);
            return Task::done(Message::Opened(rom_path, rom, Target::Transient).into());
        }

        Message::Opened(rom_path, rom, target) => {
            let sha1 = library::hasheous::rom_sha1(&rom);
            let claimed = system::family_for_media(&rom_path, &rom, app.catalogue.platform(&sha1));
            let entry = library::find_by_sha1(&sha1).map(|(_, entry)| entry);
            // Media no family claims still has a system if the user named one
            // for it before; that word is on its library entry.
            let platform = claimed
                .map(|family| family.platform)
                .or_else(|| entry.as_ref().and_then(|entry| entry.platform));
            let title = entry
                .as_ref()
                .map(|entry| entry.display_title())
                .unwrap_or_else(|| load::file_stem_title(&rom_path));
            let overrides = match target {
                Target::Library => entry.map(|entry| entry.overrides).unwrap_or_default(),
                Target::Transient => LaunchValues::default(),
            };
            let mut window = Window {
                rom_path,
                rom,
                sha1,
                title,
                platform,
                claimed: claimed.is_some(),
                overrides,
                rows: Vec::new(),
                facts: Facts::default(),
                target,
                error: None,
            };
            refresh(&mut window, app);
            app.launch_window = Some(window);
            // Whatever offered this window has been taken up.
            app.notice = None;
        }

        Message::SelectSystem(platform) => {
            if let Some(mut window) = app.launch_window.take() {
                window.platform = Some(platform);
                window.error = None;
                refresh(&mut window, app);
                app.launch_window = Some(window);
            }
        }

        Message::Set(surface, edit) => apply_edit(app, surface, &edit),

        Message::Launch => return load::launch_from_window(app),

        Message::Close => {
            app.launch_window = None;
        }
    }

    Task::none()
}

/// Publish this window's rows again and re-read what fills them; both follow
/// the values, and the family's own answer changes when the user names a
/// different system.
fn refresh(window: &mut Window, app: &App) {
    let (rows, facts) = match window.family() {
        Some(family) => plan(
            family,
            &window.rom,
            &window.overrides,
            &window.sha1,
            &app.launch_sources(),
        ),
        None => (Vec::new(), Facts::default()),
    };
    window.rows = rows.into_iter().filter(rendered).collect();
    window.facts = facts;
}

/// Put the user's word where the surface it was made on keeps it: in the
/// window for this launch, or in the library entry for every launch after.
fn apply_edit(app: &mut App, surface: EditSurface, edit: &Edit) {
    match surface {
        EditSurface::Window => {
            let Some(mut window) = app.launch_window.take() else {
                return;
            };
            edit.apply(&mut window.overrides);
            window.error = None;
            refresh(&mut window, app);
            let persist = (window.target == Target::Library)
                .then(|| (window.sha1.clone(), window.overrides.clone()));
            app.launch_window = Some(window);
            if let Some((sha1, overrides)) = persist {
                store_overrides(app, &sha1, overrides);
            }
        }
        EditSurface::GameSettings => {
            let Some(sha1) = app.viewing_sha1().map(str::to_owned) else {
                return;
            };
            let Some((_, entry)) = library::find_by_sha1(&sha1) else {
                return;
            };
            let mut overrides = entry.overrides;
            edit.apply(&mut overrides);
            store_overrides(app, &sha1, overrides);
        }
    }
}

/// Write a game's overrides to its library entry, and let everything showing it
/// pick the change up.
fn store_overrides(app: &mut App, sha1: &str, overrides: LaunchValues) {
    let Some((game_dir, mut entry)) = library::find_by_sha1(sha1) else {
        return;
    };
    entry.overrides = overrides;
    library::save_entry(&game_dir, &entry);
    app.store.notify_metadata_changed(sha1);
    if let Some(current) = &mut app.current_game
        && current.entry.sha1 == sha1
    {
        current.entry = entry;
    }
}

/// A library game's media and the family that claims it, held for the rows that
/// family publishes for it. The rows follow the user's own values, so they are
/// published again from the media on every edit rather than settled once.
#[derive(Clone)]
pub struct Media {
    family: &'static FamilyDescriptor,
    rom: Vec<u8>,
}

/// A library game's own media, read once when the game's details page opens.
/// `None` where no family is registered for the game.
pub(in crate::app) fn media(app: &App, sha1: &str) -> Option<Media> {
    let entry = app.store.entry(sha1)?;
    let family = entry.platform.and_then(system::family_of)?;
    Some(Media {
        family,
        rom: entry
            .rom_paths
            .iter()
            .find(|path| path.exists())
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default(),
    })
}

/// The rows a library game's own settings section shows, for the values it is
/// set to launch with.
pub(in crate::app) fn game_settings<'a>(
    app: &'a App,
    sha1: &str,
    media: &Media,
) -> Option<view::PanelData<'a>> {
    let entry = app.store.entry(sha1)?;
    let overrides = entry.overrides.clone();
    let (descriptors, facts) = plan(
        media.family,
        &media.rom,
        &overrides,
        sha1,
        &app.launch_sources(),
    );
    Some(view::PanelData {
        descriptors: descriptors.into_iter().filter(rendered).collect(),
        facts,
        overrides,
        firmware: &app.firmware,
        surface: EditSurface::GameSettings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::firmware::{FirmwareImage, FirmwareNeed, FirmwareSlot, sha256_hex};
    use missingno_core::launch::{LaunchChoice, LaunchOptionKind};
    use missingno_gb::firmware::DMG_BOOT_ROM;
    use missingno_gbc::firmware::CGB_BOOT_ROM;

    /// A socket over one synthetic image, hashed from the bytes the tests write
    /// — no dump is needed to exercise the folder.
    const TEST_SOCKET: &str = "test-boot-rom";

    fn socket_image() -> Vec<u8> {
        vec![0x31, 0xFE, 0xFF, 0xAF]
    }

    fn test_socket() -> FirmwareSlot {
        FirmwareSlot {
            id: TEST_SOCKET,
            label: "Test boot ROM",
            need: FirmwareNeed::Optional,
            size: 4,
            images: Box::leak(Box::new([FirmwareImage::official(
                "first",
                "First",
                sha256_hex(&socket_image()).leak(),
            )])),
        }
    }

    /// A family publishing that socket and nothing else.
    fn socket_family() -> FamilyDescriptor {
        FamilyDescriptor {
            platform: Platform::GameBoy,
            extensions: &[],
            controls: system::ControlMap::new(&[], &[], &[]),
            is_rom: |_, _| false,
            title_from_rom: |_| None,
            create_console: |_| Err("no console in a test".to_string()),
            options: |_, _| {
                vec![LaunchOptionDescriptor {
                    id: TEST_SOCKET,
                    label: "Test boot ROM",
                    kind: LaunchOptionKind::Firmware {
                        slot: test_socket(),
                    },
                }]
            },
            stated_by_media: |_| Vec::new(),
            firmware: || vec![test_socket()],
            port_config: |_| Vec::new(),
            trace: None,
        }
    }

    /// A firmware folder holding the synthetic image under an arbitrary name.
    fn stocked_library(name: &str) -> FirmwareLibrary {
        let dir = std::env::temp_dir().join(format!(
            "missingno-launch-firmware-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary firmware folder");
        std::fs::write(dir.join("anything.bin"), socket_image()).expect("the image written");
        FirmwareLibrary::scan(dir, &[test_socket()])
    }

    fn socket_facts(library: &FirmwareLibrary, defaults: &BTreeMap<String, String>) -> Facts {
        let catalogue = Catalogue::load();
        let sources = LaunchSources {
            catalogue: &catalogue,
            firmware: library,
            defaults,
            cli_firmware: None,
        };
        plan(
            &socket_family(),
            &[],
            &LaunchValues::default(),
            "",
            &sources,
        )
        .1
    }

    #[test]
    fn a_settings_default_fills_a_firmware_socket() {
        let library = stocked_library("default");
        let defaults = BTreeMap::from([(TEST_SOCKET.to_string(), "first".to_string())]);
        assert_eq!(
            socket_facts(&library, &defaults).get(TEST_SOCKET),
            Some(&LaunchValue::Firmware(FirmwareValue::Image(
                "first".to_string()
            )))
        );
    }

    #[test]
    fn an_optional_socket_with_no_default_is_left_empty() {
        let library = stocked_library("no-default");
        assert_eq!(
            socket_facts(&library, &BTreeMap::new()).get(TEST_SOCKET),
            None
        );
    }

    #[test]
    fn the_users_own_word_on_a_socket_wins_over_the_default() {
        let library = stocked_library("override");
        let defaults = BTreeMap::from([(TEST_SOCKET.to_string(), "first".to_string())]);
        let facts = socket_facts(&library, &defaults);

        let mut overrides = LaunchValues::default();
        overrides.set_firmware(TEST_SOCKET, FirmwareValue::None);
        let descriptors = (socket_family().options)(&[], &LaunchValues::default());
        assert_eq!(
            resolve(&descriptors, &overrides, &facts).firmware(TEST_SOCKET),
            Some(&FirmwareValue::None)
        );
    }

    fn descriptors() -> Vec<LaunchOptionDescriptor> {
        vec![
            LaunchOptionDescriptor {
                id: "board",
                label: "Cartridge board",
                kind: LaunchOptionKind::Choice {
                    choices: vec![
                        LaunchChoice {
                            value: "F8",
                            label: "F8",
                        },
                        LaunchChoice {
                            value: "F6",
                            label: "F6",
                        },
                    ],
                },
            },
            LaunchOptionDescriptor {
                id: "overdump",
                label: "Overdump",
                kind: LaunchOptionKind::Toggle,
            },
        ]
    }

    fn catalogued_board(code: &str) -> Facts {
        let mut facts = Facts::default();
        facts.set("board", LaunchValue::Choice(code.to_owned()));
        facts
    }

    #[test]
    fn a_catalogue_fact_fills_an_option_the_user_left_alone() {
        let values = resolve(
            &descriptors(),
            &LaunchValues::default(),
            &catalogued_board("F8"),
        );
        assert_eq!(values.choice("board"), Some("F8"));
    }

    #[test]
    fn the_users_own_word_wins_over_the_catalogues() {
        let mut overrides = LaunchValues::default();
        overrides.set_choice("board", "F6");
        let values = resolve(&descriptors(), &overrides, &catalogued_board("F8"));
        assert_eq!(values.choice("board"), Some("F6"));
    }

    #[test]
    fn an_option_nothing_names_stays_absent() {
        let values = resolve(&descriptors(), &LaunchValues::default(), &Facts::default());
        assert_eq!(values.choice("board"), None);
        assert!(values.is_empty());
    }

    #[test]
    fn an_override_for_an_option_this_system_lacks_is_dropped() {
        let mut overrides = LaunchValues::default();
        overrides.set_choice("tv-standard", "pal");
        overrides.set_toggle("overdump", true);
        let values = resolve(&descriptors(), &overrides, &Facts::default());
        assert_eq!(values.choice("tv-standard"), None);
        assert!(values.toggle("overdump"));
    }

    #[test]
    fn an_option_no_surface_renders_still_resolves() {
        let mut facts = Facts::default();
        facts.set(system::vcs::OVERDUMP, LaunchValue::Toggle(true));
        let values = resolve(&descriptors(), &LaunchValues::default(), &facts);
        assert!(values.toggle(system::vcs::OVERDUMP));
        let shown: Vec<_> = descriptors()
            .into_iter()
            .filter(rendered)
            .map(|descriptor| descriptor.id)
            .collect();
        assert_eq!(shown, ["board"]);
    }

    #[test]
    fn clearing_a_stated_board_hands_it_back_to_the_catalogue() {
        let stated = BoardValue::new("F6").with_toggle("superchip", true);
        let mut overrides = LaunchValues::default();
        Edit::Board("board", Some(stated.clone())).apply(&mut overrides);
        assert_eq!(overrides.board("board"), Some(&stated));

        Edit::Board("board", None).apply(&mut overrides);
        assert!(overrides.is_empty());

        let mut facts = Facts::default();
        facts.set("board", LaunchValue::Board(BoardValue::new("F8")));
        let values = resolve(&descriptors(), &overrides, &facts);
        assert_eq!(
            values.board("board").map(|board| board.board.as_str()),
            Some("F8")
        );
    }

    fn release_stating(enhancements: Option<Vec<Enhancement>>) -> CatalogueRelease {
        CatalogueRelease {
            title: None,
            date: None,
            publisher: None,
            tv_format: None,
            cart_type: None,
            peripherals: Vec::new(),
            enhancements,
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn a_stated_enhancement_list_names_the_console() {
        assert_eq!(
            stated_runner(&release_stating(Some(vec![Enhancement::GameBoyColor]))),
            Some("cgb")
        );
        assert_eq!(
            stated_runner(&release_stating(Some(vec![Enhancement::SuperGameBoy]))),
            Some("dmg")
        );
        assert_eq!(stated_runner(&release_stating(Some(vec![]))), Some("dmg"));
    }

    #[test]
    fn an_unstated_enhancement_list_leaves_the_console_to_the_media() {
        assert_eq!(stated_runner(&release_stating(None)), None);
    }

    /// A Game Boy image whose header carries a CGB flag at $0143 and an SGB
    /// flag at $0146.
    fn gb_rom(cgb_flag: u8, sgb_flag: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x143] = cgb_flag;
        rom[0x146] = sgb_flag;
        rom
    }

    fn media_facts(rom: &[u8]) -> Facts {
        let mut facts = Facts::default();
        for stated in system::gb::stated_by_media(rom) {
            facts.set(stated.option, stated.value);
        }
        facts
    }

    fn stated_flags(facts: &Facts) -> Option<Vec<&str>> {
        match facts.get(system::gb::ENHANCEMENTS)? {
            LaunchValue::Flags(flags) => Some(flags.iter().map(String::as_str).collect()),
            _ => None,
        }
    }

    #[test]
    fn a_stated_enhancement_list_replaces_the_headers_flags() {
        let mut facts = media_facts(&gb_rom(0x80, 0x03));
        assert_eq!(stated_flags(&facts), Some(vec!["cgb", "sgb"]));

        stated_by_release(
            &mut facts,
            &release_stating(Some(vec![Enhancement::SuperGameBoy])),
        );
        assert_eq!(stated_flags(&facts), Some(vec!["sgb"]));
    }

    #[test]
    fn a_release_stating_no_enhancements_states_a_set_with_none_in_it() {
        let mut facts = media_facts(&gb_rom(0x80, 0x03));
        stated_by_release(&mut facts, &release_stating(Some(vec![])));
        assert_eq!(stated_flags(&facts), Some(vec![]));

        let rom = gb_rom(0x80, 0x03);
        let values = resolve(
            &system::gb::launch_options(&rom, &LaunchValues::default()),
            &LaunchValues::default(),
            &facts,
        );
        assert_eq!(
            system::gb::RunnerPreference::from_launch(&values),
            Ok(system::gb::RunnerPreference::Dmg)
        );
    }

    #[test]
    fn stated_enhancements_take_the_console_off_the_header() {
        let rom = gb_rom(0x80, 0x03);
        let descriptors = system::gb::launch_options(&rom, &LaunchValues::default());
        let facts = media_facts(&rom);
        assert_eq!(
            resolve(&descriptors, &LaunchValues::default(), &facts).choice(system::gb::RUNNER),
            Some("cgb")
        );

        // The user's own set answers the console, so the header's word on it
        // is not left standing beside it.
        let mut overrides = LaunchValues::default();
        overrides.set_flags(system::gb::ENHANCEMENTS, BTreeSet::new());
        let values = resolve(&descriptors, &overrides, &facts);
        assert_eq!(values.choice(system::gb::RUNNER), None);
        assert_eq!(
            system::gb::RunnerPreference::from_launch(&values),
            Ok(system::gb::RunnerPreference::Dmg)
        );

        // A console the user named themselves still stands.
        overrides.set_choice(system::gb::RUNNER, "cgb");
        let values = resolve(&descriptors, &overrides, &facts);
        assert_eq!(values.choice(system::gb::RUNNER), Some("cgb"));
        assert_eq!(
            system::gb::RunnerPreference::from_launch(&values),
            Ok(system::gb::RunnerPreference::Cgb)
        );
    }

    /// The Game Boy family as the app registers it.
    fn gb_family() -> &'static FamilyDescriptor {
        system::family_of(Platform::GameBoy).expect("the Game Boy family is registered")
    }

    /// The rows a launch surface shows for this media under these values.
    fn rendered_rows(rom: &[u8], overrides: &LaunchValues) -> Vec<LaunchOptionDescriptor> {
        (gb_family().options)(rom, overrides)
            .into_iter()
            .filter(rendered)
            .collect()
    }

    /// The firmware sockets the family offers for this media under these values.
    fn firmware_rows(rom: &[u8], overrides: &LaunchValues) -> Vec<&'static str> {
        rendered_rows(rom, overrides)
            .into_iter()
            .filter(|descriptor| matches!(descriptor.kind, LaunchOptionKind::Firmware { .. }))
            .map(|descriptor| descriptor.id)
            .collect()
    }

    #[test]
    fn a_dual_compatible_cartridge_offers_the_socket_the_header_leads_to() {
        assert_eq!(
            firmware_rows(&gb_rom(0x80, 0x00), &LaunchValues::default()),
            [CGB_BOOT_ROM]
        );
    }

    #[test]
    fn choosing_the_other_console_swaps_the_firmware_row() {
        let mut overrides = LaunchValues::default();
        overrides.set_choice(system::gb::RUNNER, "dmg");
        assert_eq!(
            firmware_rows(&gb_rom(0x80, 0x00), &overrides),
            [DMG_BOOT_ROM]
        );
    }

    #[test]
    fn a_socket_the_chosen_console_does_not_read_is_left_aside() {
        let rom = gb_rom(0x80, 0x00);
        let mut overrides = LaunchValues::default();
        overrides.set_firmware(DMG_BOOT_ROM, FirmwareValue::Image("dmg".to_owned()));

        assert_eq!(firmware_rows(&rom, &overrides), [CGB_BOOT_ROM]);
        let descriptors = rendered_rows(&rom, &overrides);
        let values = resolve(&descriptors, &overrides, &Facts::default());
        assert_eq!(values.firmware(DMG_BOOT_ROM), None);
        // The word stays on the entry for the console that reads it.
        assert_eq!(
            overrides.firmware(DMG_BOOT_ROM),
            Some(&FirmwareValue::Image("dmg".to_owned()))
        );
    }

    #[test]
    fn the_users_console_wins_over_a_stated_enhancement_list() {
        let descriptors = [LaunchOptionDescriptor {
            id: system::gb::RUNNER,
            label: "Console",
            kind: LaunchOptionKind::Choice {
                choices: vec![
                    LaunchChoice {
                        value: "dmg",
                        label: "Game Boy (DMG)",
                    },
                    LaunchChoice {
                        value: "cgb",
                        label: "Game Boy Color (CGB)",
                    },
                ],
            },
        }];
        let stated = stated_runner(&release_stating(Some(vec![]))).unwrap();
        let mut facts = Facts::default();
        facts.set(system::gb::RUNNER, LaunchValue::Choice(stated.to_owned()));
        assert_eq!(
            resolve(&descriptors, &LaunchValues::default(), &facts).choice(system::gb::RUNNER),
            Some("dmg")
        );

        let mut overrides = LaunchValues::default();
        overrides.set_choice(system::gb::RUNNER, "cgb");
        assert_eq!(
            resolve(&descriptors, &overrides, &facts).choice(system::gb::RUNNER),
            Some("cgb")
        );
    }

    #[test]
    fn clearing_an_option_hands_it_back_to_the_catalogue() {
        let mut overrides = LaunchValues::default();
        Edit::Choice("board", Some("F6".into())).apply(&mut overrides);
        Edit::Choice("board", None).apply(&mut overrides);
        assert!(overrides.is_empty());
        let values = resolve(&descriptors(), &overrides, &catalogued_board("F8"));
        assert_eq!(values.choice("board"), Some("F8"));
    }
}
