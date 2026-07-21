//! Every tagged element's id, defined once so view tagging and the registry
//! cannot drift apart. Ids are dot-namespaced stable strings.

pub const ACTION_BAR_MENU: &str = "action_bar.menu";
pub const ACTION_BAR_BACK: &str = "action_bar.back";
pub const ACTION_BAR_HOMEBREW: &str = "action_bar.homebrew";

pub const LIBRARY_SEARCH: &str = "library.search";
pub const LIBRARY_FILTER: &str = "library.filter";
pub const LIBRARY_SORT: &str = "library.sort";
pub const LIBRARY_VIEW_GRID: &str = "library.view_grid";
pub const LIBRARY_VIEW_LIST: &str = "library.view_list";

// The action-bar overlay menu. Its items are only on screen while the menu is
// open, so the registry enumerates them only then.
pub const MENU_DISMISS: &str = "menu.dismiss";
pub const MENU_OPEN_ROM: &str = "menu.open_rom";
pub const MENU_SETTINGS: &str = "menu.settings";
pub const MENU_IMPORT_SAVE: &str = "menu.import_save";
pub const MENU_OPEN_FOLDER: &str = "menu.open_folder";
pub const MENU_REFRESH_METADATA: &str = "menu.refresh_metadata";
pub const MENU_REMOVE_GAME: &str = "menu.remove_game";
pub const MENU_DEBUGGER: &str = "menu.debugger";
pub const MENU_STEP_FRAME: &str = "menu.step_frame";
pub const MENU_RESET: &str = "menu.reset";
pub const MENU_SCREENSHOT: &str = "menu.screenshot";
pub const MENU_CAPTURE_TRACE: &str = "menu.capture_trace";

// The modal confirmation dialog, enumerated only while it is up.
pub const CONFIRM_ACCEPT: &str = "confirm.accept";
pub const CONFIRM_CANCEL: &str = "confirm.cancel";

// The game detail screen (renders its own header, not the action bar).
pub const DETAIL_BACK: &str = "detail.back";
pub const DETAIL_MENU: &str = "detail.menu";
pub const DETAIL_PLAY: &str = "detail.play";
pub const DETAIL_STOP: &str = "detail.stop";
pub const DETAIL_CARTRIDGE: &str = "detail.cartridge";

pub const CARTRIDGE_BACK: &str = "cartridge.back";
pub const FLASH_DONE: &str = "flash.done";

pub const GALLERY_EXPORT: &str = "gallery.export";

pub const HOMEBREW_SEARCH: &str = "homebrew.search";

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
