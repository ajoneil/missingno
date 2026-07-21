//! Every tagged element's id, defined once so view tagging and the registry
//! cannot drift apart. Ids are dot-namespaced stable strings.

pub const ACTION_BAR_MENU: &str = "action_bar.menu";
pub const ACTION_BAR_BACK: &str = "action_bar.back";

pub const LIBRARY_SEARCH: &str = "library.search";
pub const LIBRARY_FILTER: &str = "library.filter";

pub const SETTINGS_BACK: &str = "settings.back";
pub const SETTINGS_EXTERNAL_CLIENTS: &str = "settings.external_clients";
pub const SETTINGS_UI_AUTOMATION: &str = "settings.ui_automation";

pub const EMULATOR_PLAY_PAUSE: &str = "emulator.play_pause";
pub const EMULATOR_BACK: &str = "emulator.back";
pub const EMULATOR_STEP: &str = "emulator.step";
pub const EMULATOR_STEP_OVER: &str = "emulator.step_over";

const GAME_PREFIX: &str = "library.game.";
const SECTION_PREFIX: &str = "settings.section.";

/// The id for a library game entry, keyed by its sha1.
pub fn game(sha1: &str) -> String {
    format!("{GAME_PREFIX}{sha1}")
}

/// The sha1 back out of a `library.game.<sha1>` id.
pub fn game_sha1(id: &str) -> Option<&str> {
    id.strip_prefix(GAME_PREFIX)
}

/// The id for a settings section nav entry, keyed by its lowercase name.
pub fn section(name: &str) -> String {
    format!("{SECTION_PREFIX}{name}")
}

/// The section name back out of a `settings.section.<name>` id.
pub fn section_name(id: &str) -> Option<&str> {
    id.strip_prefix(SECTION_PREFIX)
}
