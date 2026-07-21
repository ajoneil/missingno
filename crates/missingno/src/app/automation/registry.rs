//! The semantics of each tagged id: its role and label, the message that
//! activates it, and the message a text edit produces. Plain match functions
//! over a narrow [`UiContext`] so they are unit-testable without an [`App`].
//!
//! [`App`]: crate::app::App

use super::UiKind;
use super::ids;
use crate::app::library::homebrew_browser;
use crate::app::library::screenshot_gallery;
use crate::app::library::view as library_view;
use crate::app::settings::view as settings_view;
use crate::app::{Message, debugger};

/// Which screen owns the action bar / on-screen controls right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Library,
    HomebrewBrowser,
    ScreenshotGallery,
    Settings,
    Emulator,
    Other,
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
    /// (sha1, title) of the library games currently listed, for enumerating
    /// and labelling game ids.
    pub games: Vec<(String, String)>,
    pub settings_section: settings_view::Section,
    pub allow_external_clients: bool,
    pub allow_ui_automation: bool,
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

/// Every id valid on the current screen, in reading order.
pub fn enumerate(ctx: &UiContext) -> Vec<String> {
    match ctx.screen {
        Screen::Library => {
            let mut ids = vec![
                ids::ACTION_BAR_MENU.to_string(),
                ids::LIBRARY_SEARCH.to_string(),
                ids::LIBRARY_FILTER.to_string(),
            ];
            ids.extend(ctx.games.iter().map(|(sha1, _)| ids::game(sha1)));
            ids
        }
        Screen::HomebrewBrowser | Screen::ScreenshotGallery => {
            vec![
                ids::ACTION_BAR_MENU.to_string(),
                ids::ACTION_BAR_BACK.to_string(),
            ]
        }
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
        Screen::Other => Vec::new(),
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
        ids::LIBRARY_SEARCH => (UiKind::TextInput, "Search library".to_string()),
        ids::LIBRARY_FILTER => (UiKind::Button, "Filter by system".to_string()),
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
        ids::ACTION_BAR_MENU => Message::ToggleMenu,
        ids::ACTION_BAR_BACK => match ctx.screen {
            Screen::HomebrewBrowser => Message::HomebrewBrowser(homebrew_browser::Message::Back),
            Screen::ScreenshotGallery => {
                Message::ScreenshotGallery(screenshot_gallery::Message::Back)
            }
            _ => return None,
        },
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

/// The message a text edit on `id` produces. `None` for ids that take no text.
pub(in crate::app) fn text_change(_ctx: &UiContext, id: &str, text: String) -> Option<Message> {
    match id {
        ids::LIBRARY_SEARCH => Some(library_view::Message::SearchChanged(text).into()),
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
            games: vec![
                ("abc123".to_string(), "Wario Land II".to_string()),
                ("def456".to_string(), "Tetris".to_string()),
            ],
            settings_section: settings_view::Section::General,
            allow_external_clients: false,
            allow_ui_automation: false,
        }
    }

    fn every_screen() -> Vec<UiContext> {
        let base = library_ctx();
        [
            Screen::Library,
            Screen::HomebrewBrowser,
            Screen::ScreenshotGallery,
            Screen::Settings,
            Screen::Emulator,
        ]
        .into_iter()
        .flat_map(|screen| {
            let base = base.clone();
            [false, true].into_iter().map(move |is_debugger| UiContext {
                screen,
                is_debugger,
                ..base.clone()
            })
        })
        .collect()
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
