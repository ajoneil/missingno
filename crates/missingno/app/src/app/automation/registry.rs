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
use crate::app::{CartridgeMessage, DetailMessage, Message, debugger, emulator, load};

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
    /// Which page the Controls section shows, and the controller type its
    /// Controllers block has tabbed to — together they decide which rows are on
    /// screen.
    pub settings_controls: settings_view::ControlsState,
    /// Where the pointer switch stands on the system the Controls section shows.
    pub settings_pointer_knob: bool,
    /// The Display section's rows, as the settings screen offers them.
    pub settings_display: settings_view::DisplayOptions,
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
    /// The play screen's Controllers section while it is on screen: the ports and
    /// the host devices playing them, whose pick lists it enumerates.
    pub controllers: Option<emulator::Controllers>,
    /// The play screen's Display panel while it is on screen: the options the
    /// running console offers, whose rows it enumerates.
    pub display: Option<settings_view::DisplayOptions>,
}

fn section_from_name(name: &str) -> Option<settings_view::Section> {
    settings_view::SECTIONS
        .iter()
        .find(|(_, candidate, _, _)| *candidate == name)
        .map(|(section, _, _, _)| *section)
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
            ids.extend(
                controllers_elements(ctx)
                    .into_iter()
                    .map(|element| element.id),
            );
            ids.extend(display_elements(ctx).into_iter().map(|element| element.id));
            ids
        }
        Screen::Settings => {
            let mut ids = vec![ids::SETTINGS_BACK.to_string()];
            ids.extend(
                settings_view::SECTIONS
                    .iter()
                    .map(|(_, name, _, _)| ids::section(name)),
            );
            if ctx.settings_section == settings_view::Section::Controls {
                ids.extend(controls_elements(ctx).into_iter().map(|element| element.id));
            }
            if ctx.settings_section == settings_view::Section::Display {
                ids.extend(
                    settings_display_elements(ctx)
                        .into_iter()
                        .map(|element| element.id),
                );
            }
            if ctx.settings_section == settings_view::Section::Developer {
                ids.push(ids::SETTINGS_EXTERNAL_CLIENTS.to_string());
                ids.push(ids::SETTINGS_UI_AUTOMATION.to_string());
            }
            ids
        }
    }
}

/// The Controls section's pressable elements for the page it is showing.
fn controls_elements(ctx: &UiContext) -> Vec<settings_view::PressableElement> {
    settings_view::controls_elements(&ctx.settings_controls, ctx.settings_pointer_knob)
}

/// The Display section's rows, as the settings screen names them.
fn settings_display_elements(ctx: &UiContext) -> Vec<settings_view::PressableElement> {
    settings_view::display_elements(&ctx.settings_display, ids::settings_display_row)
}

/// The Display panel's rows, empty unless it is on screen.
fn display_elements(ctx: &UiContext) -> Vec<settings_view::PressableElement> {
    match &ctx.display {
        Some(options) => settings_view::display_elements(options, ids::display_row),
        None => Vec::new(),
    }
}

/// The role and label of one settings element, on whichever surface offers it.
fn element_described(
    elements: Vec<settings_view::PressableElement>,
    id: &str,
) -> Option<(UiKind, String)> {
    elements
        .into_iter()
        .find(|element| element.id == id)
        .map(|element| {
            let kind = if element.toggle {
                UiKind::Toggle
            } else {
                UiKind::Button
            };
            (kind, element.label)
        })
}

fn element_activation(elements: Vec<settings_view::PressableElement>, id: &str) -> Option<Message> {
    elements
        .into_iter()
        .find(|element| element.id == id)
        .map(|element| element.message.into())
}

/// The Controllers section's pick lists, empty unless it is on screen.
fn controllers_elements(ctx: &UiContext) -> Vec<emulator::ControllersElement> {
    match &ctx.controllers {
        Some(controllers) => emulator::controllers_elements(controllers),
        None => Vec::new(),
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
        return settings_view::SECTIONS
            .iter()
            .find(|(_, candidate, _, _)| *candidate == name)
            .map(|(section, _, _, title)| {
                let mut label = format!("Open {title} settings");
                if *section == ctx.settings_section {
                    label.push_str(" (current)");
                }
                (UiKind::Button, label)
            });
    }
    if ids::is_controls(id) {
        return element_described(controls_elements(ctx), id);
    }
    if ids::is_controllers(id) {
        return controllers_elements(ctx)
            .into_iter()
            .find(|element| element.id == id)
            .map(|element| (UiKind::Button, element.label));
    }
    if ids::is_settings_display(id) {
        return element_described(settings_display_elements(ctx), id);
    }
    if ids::is_display(id) {
        return element_described(display_elements(ctx), id);
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
    if ids::is_controls(id) {
        return element_activation(controls_elements(ctx), id);
    }
    if ids::is_settings_display(id) {
        return element_activation(settings_display_elements(ctx), id);
    }
    if ids::is_display(id) {
        return element_activation(display_elements(ctx), id);
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
    use missingno_gb::ppu::types::palette::PaletteChoice;

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
            settings_controls: settings_view::ControlsState::default(),
            settings_pointer_knob: true,
            settings_display: settings_view::DisplayOptions {
                effects: settings_view::Effects {
                    persistence: true,
                    scanlines: true,
                    pixel_grid: true,
                },
                technology: None,
                sgb_colors: Some(true),
                palette: Some(PaletteChoice::default()),
            },
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
            controllers: None,
            display: None,
        }
    }

    /// A Game Boy as the Display panel sees it: an LCD screen, and a monochrome
    /// game whose cartridge asks for Super Game Boy colours.
    fn dmg_display() -> settings_view::DisplayOptions {
        settings_view::DisplayOptions {
            technology: Some(missingno_core::video::DisplayTechnology::Lcd {
                native: (160, 144),
                panel: missingno_core::video::LcdPanel::PassiveStn,
                pixel_aspect: 1.0,
            }),
            sgb_colors: Some(false),
            palette: Some(PaletteChoice::default()),
            ..library_ctx().settings_display
        }
    }

    /// A VCS as the Display panel sees it: a CRT, and no colour choice to make.
    fn vcs_display() -> settings_view::DisplayOptions {
        settings_view::DisplayOptions {
            technology: Some(missingno_core::video::DisplayTechnology::Crt {
                standard: missingno_core::tv::TvStandard::Ntsc,
                pixel_aspect: 12.0 / 7.0,
            }),
            sgb_colors: None,
            palette: None,
            ..library_ctx().settings_display
        }
    }

    /// A VCS as the play screen sees it: both jacks with the controllers they
    /// take, and the keyboard playing the right one. A connected pad cannot be
    /// fabricated here — only gilrs mints its id — so the keyboard stands alone.
    fn vcs_controllers() -> emulator::Controllers {
        use missingno_core::ports::Provider;
        use missingno_vcs::debug::{JOYSTICK, PORTS, RIGHT_PORT};

        emulator::Controllers {
            ports: PORTS
                .iter()
                .map(|port| emulator::PortSeat {
                    port: port.port,
                    label: port.label,
                    choices: port
                        .accepts
                        .iter()
                        .filter(|peripheral| peripheral.provider == Provider::Console)
                        .map(|peripheral| emulator::ControllerChoice {
                            peripheral: peripheral.id,
                            label: peripheral.label,
                        })
                        .collect(),
                    plugged: Some(JOYSTICK),
                })
                .collect(),
            devices: vec![emulator::DeviceSeat {
                source: crate::app::controls::InputSource::Keyboard,
                name: "Keyboard".to_string(),
                port: RIGHT_PORT,
            }],
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
        // Every settings section renders its own controls.
        for (section, ..) in settings_view::SECTIONS {
            ctxs.push(UiContext {
                screen: Screen::Settings,
                settings_section: section,
                ..base.clone()
            });
        }
        // Each Controls page renders its own rows, and the Controllers tab
        // decides which controller type's rows those are.
        for page in settings_view::controls_pages() {
            ctxs.push(UiContext {
                screen: Screen::Settings,
                settings_section: settings_view::Section::Controls,
                settings_controls: settings_view::ControlsState {
                    page,
                    ..Default::default()
                },
                ..base.clone()
            });
        }
        for family in crate::app::system::FAMILIES {
            for port in family.controls.ports {
                for peripheral in port.accepts {
                    ctxs.push(UiContext {
                        screen: Screen::Settings,
                        settings_section: settings_view::Section::Controls,
                        settings_controls: settings_view::ControlsState {
                            page: settings_view::ControlsPage::System(family.platform),
                            controller_tabs: settings_view::ControllerTabs::from([(
                                family.platform,
                                peripheral.id,
                            )]),
                        },
                        ..base.clone()
                    });
                }
            }
        }
        // The play screen with the Controllers section showing.
        ctxs.push(UiContext {
            screen: Screen::Emulator,
            controllers: Some(vcs_controllers()),
            ..base.clone()
        });
        // The play screen with the Display panel showing, on each screen type.
        for display in [dmg_display(), vcs_display()] {
            ctxs.push(UiContext {
                screen: Screen::Emulator,
                display: Some(display),
                ..base.clone()
            });
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
        // other means, so they legitimately answer neither verb. The Controllers
        // section is pick lists throughout.
        let pickers = [ids::LIBRARY_FILTER, ids::LIBRARY_SORT];
        for ctx in every_screen() {
            for id in enumerate(&ctx) {
                if pickers.contains(&id.as_str()) || ids::is_controllers(&id) {
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

    // Two elements sharing an id would collide in the bounds walk, so no screen
    // may list one twice.
    #[test]
    fn enumerated_ids_are_unique() {
        for ctx in every_screen() {
            let ids = enumerate(&ctx);
            let unique: std::collections::HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "duplicate id on {:?}", ctx.screen);
        }
    }

    // The Controllers section lists a pick list per port and one per host device,
    // named as the port descriptors and the drivers name them. It is on screen
    // only while the panel is open, so nothing is listed without it.
    #[test]
    fn the_controllers_section_lists_each_port_and_device() {
        let mut ctx = library_ctx();
        ctx.screen = Screen::Emulator;
        assert!(
            enumerate(&ctx).iter().all(|id| !ids::is_controllers(id)),
            "Controllers ids listed with the panel closed"
        );

        ctx.controllers = Some(vcs_controllers());
        let ids = enumerate(&ctx);
        let port = "emulator.controllers.port1";
        let keyboard = "emulator.controllers.device.keyboard";
        assert!(ids.contains(&port.to_string()));
        assert!(ids.contains(&keyboard.to_string()));
        assert_eq!(
            describe(&ctx, port).map(|(_, label)| label),
            Some("Choose the Right controller".to_string())
        );
        assert_eq!(
            describe(&ctx, keyboard).map(|(_, label)| label),
            Some("Choose the port Keyboard plays".to_string())
        );
    }

    // The Display panel lists the effects the running console's screen shows and
    // the colour options its games carry — nothing for another screen type. It is
    // on screen only while the panel is open.
    #[test]
    fn the_display_panel_lists_the_running_screens_options() {
        let mut ctx = library_ctx();
        ctx.screen = Screen::Emulator;
        assert!(
            enumerate(&ctx).iter().all(|id| !ids::is_display(id)),
            "Display ids listed with the panel closed"
        );

        ctx.display = Some(dmg_display());
        let listed = enumerate(&ctx);
        let grid = "emulator.display.effects.pixel_grid";
        assert!(listed.contains(&"emulator.display.effects.persistence".to_string()));
        assert!(listed.contains(&grid.to_string()));
        assert!(!listed.contains(&"emulator.display.effects.scanlines".to_string()));
        assert!(listed.contains(&"emulator.display.game_boy.sgb_colors".to_string()));
        assert!(listed.contains(&"emulator.display.game_boy.palette.original".to_string()));
        assert_eq!(
            describe(&ctx, grid),
            Some((UiKind::Toggle, "Pixel grid".to_string()))
        );
        assert!(matches!(
            activation(&ctx, grid),
            Some(Message::Settings(settings_view::Message::SetPixelGrid(
                false
            )))
        ));

        ctx.display = Some(vcs_display());
        let listed = enumerate(&ctx);
        assert!(listed.contains(&"emulator.display.effects.scanlines".to_string()));
        assert!(!listed.contains(&grid.to_string()));
        assert!(listed.iter().all(|id| !id.contains("game_boy")));
    }

    // The settings Display section lists every effect whatever is loaded — the
    // captions there say which screens each reaches — plus the colour group.
    #[test]
    fn the_display_settings_list_every_effect() {
        let mut ctx = library_ctx();
        ctx.screen = Screen::Settings;
        ctx.settings_section = settings_view::Section::Display;
        let listed = enumerate(&ctx);
        for row in ["persistence", "scanlines", "pixel_grid"] {
            assert!(listed.contains(&format!("settings.display.effects.{row}")));
        }
        let sgb = "settings.display.game_boy.sgb_colors";
        assert!(listed.contains(&sgb.to_string()));
        assert_eq!(
            describe(&ctx, sgb),
            Some((UiKind::Toggle, "Super Game Boy colours".to_string()))
        );
        let palette = "settings.display.game_boy.palette.greyscale";
        assert_eq!(
            describe(&ctx, palette),
            Some((UiKind::Button, "Use the Greyscale palette".to_string()))
        );

        ctx.settings_section = settings_view::Section::General;
        assert!(
            enumerate(&ctx)
                .iter()
                .all(|id| !ids::is_settings_display(id)),
            "Display rows listed from another section"
        );
    }

    // The Controllers block's rows are the tabbed controller type's own controls,
    // named as its descriptor names them, and listed once however many ports take
    // that type. A knob has no binding of its own — what turns it does.
    #[test]
    fn a_controller_tab_lists_the_controllers_own_controls() {
        use missingno_vcs::debug::{KEYPAD, PADDLES};

        let vcs_page = |peripheral| UiContext {
            screen: Screen::Settings,
            settings_section: settings_view::Section::Controls,
            settings_controls: settings_view::ControlsState {
                page: settings_view::ControlsPage::System(crate::app::system::Platform::AtariVcs),
                controller_tabs: settings_view::ControllerTabs::from([(
                    crate::app::system::Platform::AtariVcs,
                    peripheral,
                )]),
            },
            ..library_ctx()
        };

        let ctx = vcs_page(KEYPAD);
        let key = "settings.controls.binding.atari_vcs.peripheral3.key4.keyboard";
        let listed = enumerate(&ctx);
        assert!(listed.contains(&key.to_string()));
        assert_eq!(listed.iter().filter(|id| *id == key).count(), 1);
        assert_eq!(
            describe(&ctx, key).map(|(_, label)| label),
            Some("Bind Keypad 5 on the keyboard".to_string())
        );

        let ctx = vcs_page(PADDLES);
        let listed = enumerate(&ctx);
        // Nothing binds the knob itself; each way of winding it does, and the
        // pointer is a switch beside them.
        assert!(!listed.contains(
            &"settings.controls.binding.atari_vcs.peripheral2.knob0.keyboard".to_string()
        ));
        let clockwise = "settings.controls.binding.atari_vcs.peripheral2.knob0.clockwise.gamepad";
        assert!(listed.contains(&clockwise.to_string()));
        assert_eq!(
            describe(&ctx, clockwise).map(|(_, label)| label),
            Some("Bind Paddles Knob clockwise on the controller".to_string())
        );
        let pointer = "settings.controls.option.atari_vcs.pointer_knob";
        assert!(listed.contains(&pointer.to_string()));
        assert_eq!(
            describe(&ctx, pointer),
            Some((UiKind::Toggle, "Turn the knob with the pointer".to_string()))
        );
    }

    // Every latching panel switch binds like any other control.
    #[test]
    fn the_console_switches_are_bindable() {
        let ctx = UiContext {
            screen: Screen::Settings,
            settings_section: settings_view::Section::Controls,
            settings_controls: settings_view::ControlsState {
                page: settings_view::ControlsPage::System(crate::app::system::Platform::AtariVcs),
                ..Default::default()
            },
            ..library_ctx()
        };
        let tv_type = "settings.controls.binding.atari_vcs.panel.toggle2.keyboard";
        assert!(enumerate(&ctx).contains(&tv_type.to_string()));
        assert_eq!(
            describe(&ctx, tv_type).map(|(_, label)| label),
            Some("Bind TV Type on the keyboard".to_string())
        );
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
