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
const CONTROLLERS_PREFIX: &str = "emulator.controllers.";
const DISPLAY_PREFIX: &str = "emulator.display.";
const SECTION_PREFIX: &str = "settings.section.";
const SETTINGS_DISPLAY_PREFIX: &str = "settings.display.";
const CONTROLS_PREFIX: &str = "settings.controls.";
const CONTROLS_PAGE_PREFIX: &str = "settings.controls.page.";
const CONTROLS_TAB_PREFIX: &str = "settings.controls.tab.";
const CONTROLS_BINDING_PREFIX: &str = "settings.controls.binding.";
const CONTROLS_OPTION_PREFIX: &str = "settings.controls.option.";

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

/// Whether `id` names something inside the Controls section, whose elements the
/// section itself enumerates from the showing page.
pub fn is_controls(id: &str) -> bool {
    id.starts_with(CONTROLS_PREFIX)
}

/// The id for a Controls page selector entry, keyed by its page name
/// (`emulator`, `game_boy`).
pub fn controls_page(page: &str) -> String {
    format!("{CONTROLS_PAGE_PREFIX}{page}")
}

/// The id for the Controllers block's controller-type tab, keyed by
/// `atari_vcs.peripheral3`.
pub fn controls_tab(tab: &str) -> String {
    format!("{CONTROLS_TAB_PREFIX}{tab}")
}

/// The id for a binding button, keyed by the control it binds and the input
/// surface it binds it on: `atari_vcs.peripheral3.key4.keyboard`.
pub fn controls_binding(binding: &str) -> String {
    format!("{CONTROLS_BINDING_PREFIX}{binding}")
}

/// The id for a Controls section switch that sets something other than a
/// binding, keyed `atari_vcs.pointer_knob`.
pub fn controls_option(option: &str) -> String {
    format!("{CONTROLS_OPTION_PREFIX}{option}")
}

/// The id for the showing Controls page's reset-to-defaults button.
pub fn controls_reset(page: &str) -> String {
    format!("{CONTROLS_PREFIX}reset.{page}")
}

/// Whether `id` names a pick list of the play screen's Controllers section,
/// whose elements that section enumerates from the running machine.
pub fn is_controllers(id: &str) -> bool {
    id.starts_with(CONTROLLERS_PREFIX)
}

/// The id for a port's controller-type pick list.
pub fn controllers_port(port: missingno_core::ports::PortId) -> String {
    format!("{CONTROLLERS_PREFIX}port{}", port.0)
}

/// The id for a host device's port pick list, keyed `keyboard` / `gamepad0`.
pub fn controllers_device(device: &str) -> String {
    format!("{CONTROLLERS_PREFIX}device.{device}")
}

/// Whether `id` names a control of the play screen's Display panel, whose rows
/// that panel enumerates from the running console.
pub fn is_display(id: &str) -> bool {
    id.starts_with(DISPLAY_PREFIX)
}

/// The id for a Display panel row, keyed by its group-qualified name
/// (`effects.persistence`, `game_boy.palette.original`).
pub fn display_row(row: &str) -> String {
    format!("{DISPLAY_PREFIX}{row}")
}

/// Whether `id` names a row of the settings Display section.
pub fn is_settings_display(id: &str) -> bool {
    id.starts_with(SETTINGS_DISPLAY_PREFIX)
}

/// The id for a settings Display row, keyed the same way as the panel's.
pub fn settings_display_row(row: &str) -> String {
    format!("{SETTINGS_DISPLAY_PREFIX}{row}")
}
