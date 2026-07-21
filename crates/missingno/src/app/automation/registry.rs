//! The semantics of each tagged id: its role and label, the message that
//! activates it, and the message a text edit produces. Plain match functions
//! over a narrow [`UiContext`] so they are unit-testable without an [`App`].
//!
//! [`App`]: crate::app::App

use super::UiKind;
use super::ids;
use crate::app::library::homebrew_browser;
use crate::app::library::screenshot_gallery;
use crate::app::library::view::{self as library_view, LibraryLayout};
use crate::app::settings::view as settings_view;
use crate::app::{CartridgeMessage, DetailMessage, Message, debugger, load};

/// Which screen owns the action bar / on-screen controls right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Library,
    GameDetail,
    CartridgeActions,
    FlashCartridge,
    HomebrewBrowser,
    ScreenshotGallery,
    Settings,
    Emulator,
}

/// The slice of [`App`] state the registry needs — extracted so a test can
/// build one directly.
///
/// [`App`]: crate::app::App
#[derive(Debug, Clone)]
pub struct UiContext {
    pub screen: Screen,
    pub running: bool,
    pub is_debugger: bool,
    /// Whether the debugger session is active — selects the emulator menu's items.
    pub debugger_enabled: bool,
    /// Whether the action-bar overlay menu is open. Its items are enumerated
    /// only while it is.
    pub menu_open: bool,
    /// The confirm button's label when the modal confirmation dialog is up;
    /// `None` when no dialog is showing.
    pub confirm_accept_label: Option<String>,
    /// (sha1, title) of the library games currently listed, for enumerating
    /// and labelling game ids.
    pub games: Vec<(String, String)>,
    pub settings_section: settings_view::Section,
    pub allow_external_clients: bool,
    pub allow_ui_automation: bool,
    pub library_layout: LibraryLayout,
    /// Whether the library offers the Browse Homebrew action bar button.
    pub homebrew_available: bool,
    /// Whether the homebrew browser is showing an entry's detail (its search
    /// field is then off screen).
    pub homebrew_entry_selected: bool,
    /// The sha1 of the game whose detail is being viewed, for detail actions.
    pub viewing_sha1: Option<String>,
    /// Detail-screen affordances: whether the viewed game has a ROM on disk,
    /// whether it is currently loaded, and whether cartridge actions are offered.
    pub detail_has_rom: bool,
    pub detail_game_loaded: bool,
    pub detail_cartridge_actions: bool,
    /// Whether a cartridge flash is mid-write (no exit button is offered then).
    pub flash_in_progress: bool,
}

const SECTIONS: [(&str, settings_view::Section); 4] = [
    ("general", settings_view::Section::General),
    ("display", settings_view::Section::Display),
    ("controls", settings_view::Section::Controls),
    ("hardware", settings_view::Section::Hardware),
];

fn section_from_name(name: &str) -> Option<settings_view::Section> {
    SECTIONS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, section)| *section)
}

fn section_label(name: &str) -> String {
    let mut chars = name.chars();
    let title = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    format!("Open {title} settings")
}

fn layout_label(base: &str, ctx: &UiContext, layout: LibraryLayout) -> String {
    if ctx.library_layout == layout {
        format!("{base} (current)")
    } else {
        base.to_string()
    }
}

/// The overlay menu items for the current screen, in reading order — mirroring
/// the per-screen branches the shell builds. Settings is always last.
fn menu_items(ctx: &UiContext) -> Vec<String> {
    let mut items: Vec<&str> = Vec::new();
    match ctx.screen {
        Screen::Library | Screen::HomebrewBrowser => items.push(ids::MENU_OPEN_ROM),
        Screen::GameDetail => items.extend([
            ids::MENU_OPEN_ROM,
            ids::MENU_IMPORT_SAVE,
            ids::MENU_OPEN_FOLDER,
            ids::MENU_REFRESH_METADATA,
            ids::MENU_REMOVE_GAME,
        ]),
        Screen::Emulator => {
            if ctx.debugger_enabled {
                items.push(ids::MENU_STEP_FRAME);
            } else {
                items.push(ids::MENU_DEBUGGER);
            }
            items.push(ids::MENU_RESET);
            items.push(ids::MENU_SCREENSHOT);
            if ctx.debugger_enabled {
                items.push(ids::MENU_CAPTURE_TRACE);
            }
        }
        _ => {}
    }
    items.push(ids::MENU_SETTINGS);
    items
        .into_iter()
        .map(str::to_string)
        .chain(std::iter::once(ids::MENU_DISMISS.to_string()))
        .collect()
}

/// Every id valid on the current screen, in reading order. Modal overlays take
/// over: a confirmation dialog or open menu masks the screen beneath it, so only
/// the overlay's own controls are enumerated.
pub fn enumerate(ctx: &UiContext) -> Vec<String> {
    if ctx.confirm_accept_label.is_some() {
        return vec![
            ids::CONFIRM_CANCEL.to_string(),
            ids::CONFIRM_ACCEPT.to_string(),
        ];
    }
    if ctx.menu_open {
        return menu_items(ctx);
    }
    match ctx.screen {
        Screen::Library => {
            let mut ids = vec![ids::ACTION_BAR_MENU.to_string()];
            if ctx.homebrew_available {
                ids.push(ids::ACTION_BAR_HOMEBREW.to_string());
            }
            ids.push(ids::LIBRARY_SEARCH.to_string());
            ids.push(ids::LIBRARY_FILTER.to_string());
            ids.push(ids::LIBRARY_SORT.to_string());
            ids.push(ids::LIBRARY_VIEW_GRID.to_string());
            ids.push(ids::LIBRARY_VIEW_LIST.to_string());
            ids.extend(ctx.games.iter().map(|(sha1, _)| ids::game(sha1)));
            ids
        }
        Screen::GameDetail => {
            let mut ids = vec![ids::DETAIL_BACK.to_string(), ids::DETAIL_MENU.to_string()];
            if ctx.detail_has_rom {
                ids.push(ids::DETAIL_PLAY.to_string());
                if ctx.detail_game_loaded {
                    ids.push(ids::DETAIL_STOP.to_string());
                }
            }
            if ctx.detail_cartridge_actions {
                ids.push(ids::DETAIL_CARTRIDGE.to_string());
            }
            ids
        }
        Screen::CartridgeActions => vec![ids::CARTRIDGE_BACK.to_string()],
        Screen::FlashCartridge => {
            if ctx.flash_in_progress {
                Vec::new()
            } else {
                vec![ids::FLASH_DONE.to_string()]
            }
        }
        Screen::HomebrewBrowser => {
            let mut ids = vec![
                ids::ACTION_BAR_MENU.to_string(),
                ids::ACTION_BAR_BACK.to_string(),
            ];
            if !ctx.homebrew_entry_selected {
                ids.push(ids::HOMEBREW_SEARCH.to_string());
            }
            ids
        }
        Screen::ScreenshotGallery => vec![
            ids::ACTION_BAR_MENU.to_string(),
            ids::ACTION_BAR_BACK.to_string(),
            ids::GALLERY_EXPORT.to_string(),
        ],
        Screen::Emulator => {
            let mut ids = vec![
                ids::EMULATOR_BACK.to_string(),
                ids::EMULATOR_PLAY_PAUSE.to_string(),
                ids::ACTION_BAR_MENU.to_string(),
            ];
            if ctx.is_debugger {
                ids.push(ids::EMULATOR_STEP.to_string());
                ids.push(ids::EMULATOR_STEP_OVER.to_string());
            }
            ids
        }
        Screen::Settings => {
            let mut ids = vec![ids::SETTINGS_BACK.to_string()];
            ids.extend(SECTIONS.iter().map(|(name, _)| ids::section(name)));
            ids.push(ids::SETTINGS_EXTERNAL_CLIENTS.to_string());
            ids.push(ids::SETTINGS_UI_AUTOMATION.to_string());
            ids
        }
    }
}

/// The role and human label for `id`, or `None` if it is not a known id in this
/// context.
pub fn describe(ctx: &UiContext, id: &str) -> Option<(UiKind, String)> {
    if let Some(sha1) = ids::game_sha1(id) {
        if sha1.is_empty() {
            return None;
        }
        let title = ctx
            .games
            .iter()
            .find(|(listed, _)| listed == sha1)
            .map(|(_, title)| title.as_str());
        return Some((
            UiKind::Button,
            match title {
                Some(title) => format!("Open {title}"),
                None => "Open game".to_string(),
            },
        ));
    }
    if let Some(name) = ids::section_name(id) {
        return section_from_name(name).map(|section| {
            let mut label = section_label(name);
            if section == ctx.settings_section {
                label.push_str(" (current)");
            }
            (UiKind::Button, label)
        });
    }
    let described = match id {
        ids::ACTION_BAR_MENU => (UiKind::Button, "Open menu".to_string()),
        ids::ACTION_BAR_BACK => {
            let label = match ctx.screen {
                Screen::ScreenshotGallery => "Back to game",
                _ => "Back to library",
            };
            (UiKind::Button, label.to_string())
        }
        ids::ACTION_BAR_HOMEBREW => (UiKind::Button, "Browse homebrew".to_string()),
        ids::LIBRARY_SEARCH => (UiKind::TextInput, "Search library".to_string()),
        ids::LIBRARY_FILTER => (UiKind::Button, "Filter by system".to_string()),
        ids::LIBRARY_SORT => (UiKind::Button, "Sort library".to_string()),
        ids::LIBRARY_VIEW_GRID => (
            UiKind::Button,
            layout_label("Show as grid", ctx, LibraryLayout::Grid),
        ),
        ids::LIBRARY_VIEW_LIST => (
            UiKind::Button,
            layout_label("Show as list", ctx, LibraryLayout::List),
        ),
        ids::MENU_DISMISS => (UiKind::Button, "Close menu".to_string()),
        ids::MENU_OPEN_ROM => (UiKind::Button, "Open ROM file".to_string()),
        ids::MENU_SETTINGS => (UiKind::Button, "Open settings".to_string()),
        ids::MENU_IMPORT_SAVE => (UiKind::Button, "Import save".to_string()),
        ids::MENU_OPEN_FOLDER => (UiKind::Button, "Open game folder".to_string()),
        ids::MENU_REFRESH_METADATA => (UiKind::Button, "Refresh metadata".to_string()),
        ids::MENU_REMOVE_GAME => (UiKind::Button, "Remove game from library".to_string()),
        ids::MENU_DEBUGGER => (UiKind::Button, "Open debugger".to_string()),
        ids::MENU_STEP_FRAME => (UiKind::Button, "Step one frame".to_string()),
        ids::MENU_RESET => (UiKind::Button, "Reset emulator".to_string()),
        ids::MENU_SCREENSHOT => (UiKind::Button, "Take screenshot".to_string()),
        ids::MENU_CAPTURE_TRACE => (UiKind::Button, "Capture trace".to_string()),
        ids::CONFIRM_ACCEPT => (
            UiKind::Button,
            ctx.confirm_accept_label
                .clone()
                .unwrap_or_else(|| "Confirm".to_string()),
        ),
        ids::CONFIRM_CANCEL => (UiKind::Button, "Cancel".to_string()),
        ids::DETAIL_BACK => (UiKind::Button, "Back to library".to_string()),
        ids::DETAIL_MENU => (UiKind::Button, "Open menu".to_string()),
        ids::DETAIL_PLAY => {
            let label = if ctx.detail_game_loaded {
                "Resume"
            } else {
                "Play"
            };
            (UiKind::Button, label.to_string())
        }
        ids::DETAIL_STOP => (UiKind::Button, "Stop".to_string()),
        ids::DETAIL_CARTRIDGE => (UiKind::Button, "Cartridge actions".to_string()),
        ids::CARTRIDGE_BACK => (UiKind::Button, "Back to game details".to_string()),
        ids::FLASH_DONE => (UiKind::Button, "Done".to_string()),
        ids::GALLERY_EXPORT => (UiKind::Button, "Export screenshot as PNG".to_string()),
        ids::HOMEBREW_SEARCH => (UiKind::TextInput, "Search homebrew".to_string()),
        ids::SETTINGS_BACK => (UiKind::Button, "Back to library".to_string()),
        ids::SETTINGS_EXTERNAL_CLIENTS => (
            UiKind::Toggle,
            "Allow external debugger clients".to_string(),
        ),
        ids::SETTINGS_UI_AUTOMATION => (UiKind::Toggle, "Allow UI automation".to_string()),
        ids::EMULATOR_PLAY_PAUSE => {
            let label = if ctx.running { "Pause" } else { "Play" };
            (UiKind::Button, label.to_string())
        }
        ids::EMULATOR_BACK => {
            let label = if ctx.is_debugger {
                "Close debugger"
            } else {
                "Back to game details"
            };
            (UiKind::Button, label.to_string())
        }
        ids::EMULATOR_STEP => (UiKind::Button, "Step".to_string()),
        ids::EMULATOR_STEP_OVER => (UiKind::Button, "Step over".to_string()),
        _ => return None,
    };
    Some(described)
}

/// Whether `id` currently accepts input. Unknown ids report `false`.
pub fn enabled(ctx: &UiContext, id: &str) -> bool {
    match id {
        ids::EMULATOR_STEP | ids::EMULATOR_STEP_OVER => !ctx.running,
        _ => describe(ctx, id).is_some(),
    }
}

/// The message that activates `id` — pressing a button, toggling a toggle.
/// `None` when the id names something with no press action (a text field).
pub(in crate::app) fn activation(ctx: &UiContext, id: &str) -> Option<Message> {
    if let Some(sha1) = ids::game_sha1(id) {
        return (!sha1.is_empty())
            .then(|| library_view::Message::SelectGame(sha1.to_string()).into());
    }
    if let Some(name) = ids::section_name(id) {
        return section_from_name(name)
            .map(|section| settings_view::Message::SelectSection(section).into());
    }
    let message = match id {
        ids::ACTION_BAR_MENU | ids::DETAIL_MENU => Message::ToggleMenu,
        ids::ACTION_BAR_BACK => match ctx.screen {
            Screen::HomebrewBrowser => Message::HomebrewBrowser(homebrew_browser::Message::Back),
            Screen::ScreenshotGallery => {
                Message::ScreenshotGallery(screenshot_gallery::Message::Back)
            }
            _ => return None,
        },
        ids::ACTION_BAR_HOMEBREW => Message::OpenHomebrewBrowser,
        ids::LIBRARY_VIEW_GRID => library_view::Message::LayoutSelected(LibraryLayout::Grid).into(),
        ids::LIBRARY_VIEW_LIST => library_view::Message::LayoutSelected(LibraryLayout::List).into(),
        ids::MENU_DISMISS => Message::DismissMenu,
        ids::MENU_OPEN_ROM => menu_action(load::Message::Pick.into()),
        ids::MENU_SETTINGS => menu_action(Message::ShowSettings),
        ids::MENU_IMPORT_SAVE => menu_action(Message::Detail(DetailMessage::ImportSave)),
        ids::MENU_OPEN_FOLDER => menu_action(Message::Detail(DetailMessage::OpenGameFolder)),
        ids::MENU_REFRESH_METADATA => menu_action(Message::Detail(DetailMessage::RefreshMetadata)),
        ids::MENU_REMOVE_GAME => menu_action(Message::Detail(DetailMessage::RemoveGame)),
        ids::MENU_DEBUGGER => menu_action(Message::ToggleDebugger(true)),
        ids::MENU_STEP_FRAME => menu_action(debugger::Message::StepFrame.into()),
        ids::MENU_RESET => menu_action(Message::Reset),
        ids::MENU_SCREENSHOT => menu_action(Message::TakeScreenshot),
        ids::MENU_CAPTURE_TRACE => menu_action(debugger::Message::CaptureFrame.into()),
        ids::CONFIRM_ACCEPT => Message::ConfirmAction,
        ids::CONFIRM_CANCEL => Message::DismissConfirm,
        ids::DETAIL_BACK => Message::BackToLibrary,
        ids::DETAIL_PLAY => Message::PlayFromDetail,
        ids::DETAIL_STOP => Message::StopGame,
        ids::DETAIL_CARTRIDGE => {
            Message::Cartridge(CartridgeMessage::ShowActions(ctx.viewing_sha1.clone()?))
        }
        ids::CARTRIDGE_BACK => Message::Cartridge(CartridgeMessage::Back),
        ids::FLASH_DONE => Message::Cartridge(CartridgeMessage::FlashCancel),
        ids::GALLERY_EXPORT => Message::ScreenshotGallery(screenshot_gallery::Message::Export),
        ids::SETTINGS_BACK => settings_view::Message::Back.into(),
        ids::SETTINGS_EXTERNAL_CLIENTS => {
            settings_view::Message::SetAllowExternalClients(!ctx.allow_external_clients).into()
        }
        ids::SETTINGS_UI_AUTOMATION => {
            settings_view::Message::SetAllowUiAutomation(!ctx.allow_ui_automation).into()
        }
        ids::EMULATOR_PLAY_PAUSE => {
            if ctx.running {
                Message::Pause
            } else {
                Message::Run
            }
        }
        ids::EMULATOR_BACK => {
            if ctx.is_debugger {
                Message::ToggleDebugger(false)
            } else {
                Message::BackToDetail
            }
        }
        ids::EMULATOR_STEP => debugger::Message::Step.into(),
        ids::EMULATOR_STEP_OVER => debugger::Message::StepOver.into(),
        _ => return None,
    };
    Some(message)
}

/// Wrap `inner` the way an overlay menu item fires: dismiss the menu, then run
/// the inner message.
fn menu_action(inner: Message) -> Message {
    Message::MenuAction(Box::new(inner))
}

/// The message a text edit on `id` produces. `None` for ids that take no text.
pub(in crate::app) fn text_change(_ctx: &UiContext, id: &str, text: String) -> Option<Message> {
    match id {
        ids::LIBRARY_SEARCH => Some(library_view::Message::SearchChanged(text).into()),
        ids::HOMEBREW_SEARCH => Some(homebrew_browser::Message::SearchTextChanged(text).into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_ctx() -> UiContext {
        UiContext {
            screen: Screen::Library,
            running: false,
            is_debugger: false,
            debugger_enabled: false,
            menu_open: false,
            confirm_accept_label: None,
            games: vec![
                ("abc123".to_string(), "Wario Land II".to_string()),
                ("def456".to_string(), "Tetris".to_string()),
            ],
            settings_section: settings_view::Section::General,
            allow_external_clients: false,
            allow_ui_automation: false,
            library_layout: LibraryLayout::Grid,
            homebrew_available: true,
            homebrew_entry_selected: false,
            viewing_sha1: Some("abc123".to_string()),
            detail_has_rom: true,
            detail_game_loaded: true,
            detail_cartridge_actions: true,
            flash_in_progress: false,
        }
    }

    const SCREENS: [Screen; 8] = [
        Screen::Library,
        Screen::GameDetail,
        Screen::CartridgeActions,
        Screen::FlashCartridge,
        Screen::HomebrewBrowser,
        Screen::ScreenshotGallery,
        Screen::Settings,
        Screen::Emulator,
    ];

    fn every_screen() -> Vec<UiContext> {
        let base = library_ctx();
        let mut ctxs = Vec::new();
        for screen in SCREENS {
            for is_debugger in [false, true] {
                for debugger_enabled in [false, true] {
                    for menu_open in [false, true] {
                        ctxs.push(UiContext {
                            screen,
                            is_debugger,
                            debugger_enabled,
                            menu_open,
                            ..base.clone()
                        });
                    }
                }
            }
        }
        // The modal confirmation dialog masks whatever is beneath it.
        ctxs.push(UiContext {
            confirm_accept_label: Some("Reset".to_string()),
            ..base.clone()
        });
        ctxs
    }

    // Every enumerated id answers `describe` — no id can be listed without a
    // role and label, so the tree never carries a nameless node.
    #[test]
    fn every_enumerated_id_is_described() {
        for ctx in every_screen() {
            for id in enumerate(&ctx) {
                assert!(
                    describe(&ctx, &id).is_some(),
                    "id {id} on {:?} has no description",
                    ctx.screen
                );
            }
        }
    }

    // A pressable id round-trips to a message; a text field declines activation
    // and instead answers text_change.
    #[test]
    fn activation_and_text_change_round_trip() {
        let ctx = library_ctx();
        assert!(activation(&ctx, &ids::game("abc123")).is_some());
        assert!(activation(&ctx, ids::LIBRARY_SEARCH).is_none());
        assert!(text_change(&ctx, ids::LIBRARY_SEARCH, "zelda".into()).is_some());
        assert!(text_change(&ctx, &ids::game("abc123"), "x".into()).is_none());
    }

    // Every enumerated id is either activatable, accepts text, or is a picker
    // (a drop-down has no single-message activation, only bounds). Nothing else
    // may be enumerated without a way to drive it.
    #[test]
    fn every_enumerated_id_is_actionable() {
        // Pick-lists are registered for their bounds; a client opens them by
        // other means, so they legitimately answer neither verb.
        let pickers = [ids::LIBRARY_FILTER, ids::LIBRARY_SORT];
        for ctx in every_screen() {
            for id in enumerate(&ctx) {
                if pickers.contains(&id.as_str()) {
                    continue;
                }
                let is_text = matches!(describe(&ctx, &id), Some((UiKind::TextInput, _)));
                if is_text {
                    assert!(
                        text_change(&ctx, &id, "x".into()).is_some(),
                        "text id {id} on {:?} takes no text",
                        ctx.screen
                    );
                } else {
                    assert!(
                        activation(&ctx, &id).is_some(),
                        "id {id} on {:?} has no activation",
                        ctx.screen
                    );
                }
            }
        }
    }

    #[test]
    fn step_disabled_while_running() {
        let mut ctx = library_ctx();
        ctx.screen = Screen::Emulator;
        ctx.is_debugger = true;
        ctx.running = true;
        assert!(!enabled(&ctx, ids::EMULATOR_STEP));
        ctx.running = false;
        assert!(enabled(&ctx, ids::EMULATOR_STEP));
    }

    #[test]
    fn unknown_id_is_undescribed() {
        assert!(describe(&library_ctx(), "no.such.id").is_none());
        assert!(describe(&library_ctx(), &ids::game("")).is_none());
    }

    #[test]
    fn game_labels_carry_the_title() {
        let (_, label) = describe(&library_ctx(), &ids::game("abc123")).unwrap();
        assert_eq!(label, "Open Wario Land II");
    }
}
