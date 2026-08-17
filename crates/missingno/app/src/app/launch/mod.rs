//! What the user decides about a ROM before it boots, in one place.
//!
//! Every row the launch window shows comes from the family's own launch
//! descriptors, so a core that publishes a new option gets a row without a
//! change here. An option the user has not set is absent — absence is what
//! leaves the decision to the core — and what the catalogue states about a
//! dump is read live at launch rather than copied onto the library entry.

use std::collections::BTreeMap;
use std::path::PathBuf;

use iced::Task;
use missingno_core::launch::{LaunchOptionDescriptor, LaunchValue, LaunchValues};
use missingno_gamedb::Controller;

use crate::app::library::catalogue::Catalogue;
use crate::app::system::{self, FamilyDescriptor, Platform};
use crate::app::{self, App, library, load};

mod view;

pub(in crate::app) use view::{PanelData, panel, window};

/// Where a value the user did not set came from, in the words the window uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FactSource {
    /// The bundled game database's entry for this dump.
    Catalogue,
    /// A header the media carries.
    Header,
    /// Named on the command line this run started with.
    CommandLine,
}

impl FactSource {
    fn word(self) -> &'static str {
        match self {
            FactSource::Catalogue => "game database",
            FactSource::Header => "header",
            FactSource::CommandLine => "command line",
        }
    }
}

/// One value something other than the user supplies for an option.
#[derive(Clone)]
pub struct Fact {
    pub value: LaunchValue,
    pub source: FactSource,
}

/// What fills the options the user has not set, keyed by option id. Sparse: an
/// option nothing here names is one only the core can resolve.
#[derive(Clone, Default)]
pub struct Facts(BTreeMap<&'static str, Fact>);

impl Facts {
    fn set(&mut self, id: &'static str, value: LaunchValue, source: FactSource) {
        self.0.insert(id, Fact { value, source });
    }

    pub fn get(&self, id: &str) -> Option<&Fact> {
        self.0.get(id)
    }
}

/// What the catalogue and the media itself state about a dump, ahead of any
/// word from the user: the game database's facts about this hash, the header's
/// own, and the boot ROM this run was started with.
pub fn facts(
    family: &FamilyDescriptor,
    rom: &[u8],
    catalogue: &Catalogue,
    sha1: &str,
    boot_rom: Option<&missingno_gb::BootRom>,
) -> Facts {
    let mut facts = Facts::default();

    for stated in (family.stated_by_media)(rom) {
        facts.set(stated.option, stated.value, FactSource::Header);
    }

    if let Some((_, release, artifact)) = catalogue.lookup_hash(sha1) {
        if let Some(standard) = release.tv_format {
            facts.set(
                system::vcs::TV_STANDARD,
                LaunchValue::Choice(standard.code().to_owned()),
                FactSource::Catalogue,
            );
        }
        // Every family publishes its board option under the same id, so one
        // key carries the catalogue's word whichever core is about to read it.
        if let Some(board) = &release.cart_type {
            facts.set(
                system::vcs::BOARD,
                LaunchValue::Choice(board.clone()),
                FactSource::Catalogue,
            );
        }
        // A dump padded past the cartridge's silicon: the stated board says
        // where the silicon ends.
        facts.set(
            system::vcs::OVERDUMP,
            LaunchValue::Toggle(artifact.defect == Some(missingno_gamedb::Defect::Overdump)),
            FactSource::Catalogue,
        );
    }

    if let Some(boot_rom) = boot_rom {
        facts.set(
            system::gb::BOOT_ROM,
            LaunchValue::File(boot_rom.bytes().to_vec()),
            FactSource::CommandLine,
        );
    }

    facts
}

/// The controllers the catalogue says this dump's release needs; empty leaves
/// the console's power-on configuration.
pub fn catalogued_controllers(catalogue: &Catalogue, sha1: &str) -> Vec<Controller> {
    catalogue
        .lookup_hash(sha1)
        .map(|(_, release, _)| release.controllers.clone())
        .unwrap_or_default()
}

/// The values a launch runs with: the user's own word on an option wins, else
/// whatever fact fills it, else nothing — which leaves the option to the core.
/// An override naming an option this family does not publish is dropped.
pub fn resolve(
    descriptors: &[LaunchOptionDescriptor],
    overrides: &LaunchValues,
    facts: &Facts,
) -> LaunchValues {
    for (id, _) in overrides.iter() {
        if !descriptors.iter().any(|descriptor| descriptor.id == id) {
            eprintln!("launch option \"{id}\" is not one this system accepts; ignoring it");
        }
    }

    let mut values = LaunchValues::default();
    for descriptor in descriptors {
        if let Some(value) = overrides
            .value(descriptor.id)
            .or_else(|| facts.get(descriptor.id).map(|fact| &fact.value))
        {
            values.set(descriptor.id, value.clone());
        }
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
    Toggle(&'static str, Option<bool>),
    File(&'static str, Option<Vec<u8>>),
}

impl Edit {
    fn apply(&self, values: &mut LaunchValues) {
        match self {
            Edit::Choice(id, Some(value)) => values.set_choice(*id, value.clone()),
            Edit::Toggle(id, Some(value)) => values.set_toggle(*id, *value),
            Edit::File(id, Some(bytes)) => values.set_file(*id, bytes.clone()),
            Edit::Choice(id, None) | Edit::Toggle(id, None) | Edit::File(id, None) => {
                values.clear(id)
            }
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
    /// Whether a family claimed the media itself; unclaimed media asks first.
    claimed: bool,
    /// The user's own word on the options, as this window has it.
    pub overrides: LaunchValues,
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
    PickFile(EditSurface, &'static str),
    FilePicked(EditSurface, &'static str, Option<rfd::FileHandle>),
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
            let claimed = system::family_for(&rom_path, &rom);
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
                facts: Facts::default(),
                target,
                error: None,
            };
            refresh_facts(&mut window, app);
            app.launch_window = Some(window);
            // Whatever offered this window has been taken up.
            app.notice = None;
        }

        Message::SelectSystem(platform) => {
            if let Some(mut window) = app.launch_window.take() {
                window.platform = Some(platform);
                window.error = None;
                refresh_facts(&mut window, app);
                app.launch_window = Some(window);
            }
        }

        Message::Set(surface, edit) => apply_edit(app, surface, &edit),

        Message::PickFile(surface, option) => {
            let dialog = rfd::AsyncFileDialog::new();
            return Task::perform(dialog.pick_file(), move |handle| {
                Message::FilePicked(surface, option, handle).into()
            });
        }

        Message::FilePicked(surface, option, handle) => {
            if let Some(handle) = handle
                && let Ok(bytes) = std::fs::read(handle.path())
            {
                apply_edit(app, surface, &Edit::File(option, Some(bytes)));
            }
        }

        Message::Launch => return load::launch_from_window(app),

        Message::Close => {
            app.launch_window = None;
        }
    }

    Task::none()
}

/// Re-read what fills this window's options; the family's own facts change when
/// the user names a different system.
fn refresh_facts(window: &mut Window, app: &App) {
    window.facts = match window.family() {
        Some(family) => facts(
            family,
            &window.rom,
            &app.catalogue,
            &window.sha1,
            app.boot_rom.as_ref(),
        ),
        None => Facts::default(),
    };
}

/// Put the user's word where the surface it was made on keeps it: in the
/// window for this launch, or in the library entry for every launch after.
fn apply_edit(app: &mut App, surface: EditSurface, edit: &Edit) {
    match surface {
        EditSurface::Window => {
            let Some(window) = &mut app.launch_window else {
                return;
            };
            edit.apply(&mut window.overrides);
            window.error = None;
            let persist = (window.target == Target::Library)
                .then(|| (window.sha1.clone(), window.overrides.clone()));
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

/// What fills a library game's launch options where the user has not. Reads the
/// media, so it is taken once when the game's details page opens.
pub(in crate::app) fn facts_for_game(app: &App, sha1: &str) -> Facts {
    let Some(entry) = app.store.entry(sha1) else {
        return Facts::default();
    };
    let Some(family) = entry.platform.and_then(system::family_of) else {
        return Facts::default();
    };
    let rom = entry
        .rom_paths
        .iter()
        .find(|path| path.exists())
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    facts(family, &rom, &app.catalogue, sha1, app.boot_rom.as_ref())
}

/// The rows a library game's own settings section shows.
pub(in crate::app) fn game_settings(
    app: &App,
    sha1: &str,
    facts: &Facts,
) -> Option<view::PanelData> {
    let entry = app.store.entry(sha1)?;
    let family = system::family_of(entry.platform?)?;
    Some(view::PanelData {
        descriptors: (family.options)(),
        overrides: entry.overrides.clone(),
        facts: facts.clone(),
        surface: EditSurface::GameSettings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::launch::{LaunchChoice, LaunchOptionKind};

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
        facts.set(
            "board",
            LaunchValue::Choice(code.to_owned()),
            FactSource::Catalogue,
        );
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
    fn clearing_an_option_hands_it_back_to_the_catalogue() {
        let mut overrides = LaunchValues::default();
        Edit::Choice("board", Some("F6".into())).apply(&mut overrides);
        Edit::Choice("board", None).apply(&mut overrides);
        assert!(overrides.is_empty());
        let values = resolve(&descriptors(), &overrides, &catalogued_board("F8"));
        assert_eq!(values.choice("board"), Some("F8"));
    }
}
