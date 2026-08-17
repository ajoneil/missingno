use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task, mouse,
    widget::{container, mouse_area, row, text as iced_text},
};

use crate::app::ui::{
    icons::{self, Icon},
    sizes::{m, s},
};
use crate::app::{App, DetailSubScreen, Game, Message, Screen, launch, library};

impl App {
    fn homebrew_enabled(&self) -> bool {
        self.settings.internet_enabled && self.settings.homebrew_hub_enabled
    }

    /// Library/Homebrew/ScreenshotGallery page content (no chrome).
    pub(super) fn page_content(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Library { .. } => self.library_view(),
            Screen::HomebrewBrowser { state } => {
                library::homebrew_browser::view(state, &self.catalogue)
            }
            Screen::ViewingGame {
                sub_screen: DetailSubScreen::ScreenshotGallery { gallery_state },
                ..
            } => library::screenshot_gallery::view(gallery_state),
            _ => unreachable!(),
        }
    }

    pub(super) fn detail_view(&self) -> Element<'_, Message> {
        let (viewing_sha1, section, hovered_log_entry, header_hovered, media_options) =
            match &self.screen {
                Screen::ViewingGame {
                    sha1,
                    sub_screen:
                        DetailSubScreen::Detail {
                            section,
                            hovered_log_entry,
                            header_hovered,
                            media_options,
                        },
                } => (
                    Some(sha1.as_str()),
                    *section,
                    *hovered_log_entry,
                    *header_hovered,
                    Some(media_options),
                ),
                _ => (None, Default::default(), None, false, None),
            };

        let sha1 = match viewing_sha1 {
            Some(s) => s,
            None => return self.library_view(),
        };

        let summary = self.store.summary(sha1);

        // The loaded game carries the fresher entry; anything else comes from
        // the store.
        let entry = match self
            .current_game
            .as_ref()
            .filter(|current| current.entry.sha1 == sha1)
        {
            Some(current) => &current.entry,
            None => match summary {
                Some(summary) => &summary.entry,
                None => return self.library_view(),
            },
        };

        // Use pre-rendered thumbnail from the store
        let cover = summary.and_then(|s| s.thumbnail.as_ref());

        let activity_state = self.store.activity_for(sha1);

        let live_session = self
            .current_game
            .as_ref()
            .filter(|c| sha1 == c.entry.sha1.as_str())
            .and_then(|c| c.session.as_ref());

        let is_loaded = self
            .current_game
            .as_ref()
            .map(|c| c.entry.sha1 == sha1 && matches!(self.game, Game::Loaded(_)))
            .unwrap_or(false);

        library::detail_view::view(library::detail_view::DetailData {
            entry,
            cover,
            activity_state,
            live_session,
            live_screenshots: self.store.live_screenshots(),
            live_prints: self.store.live_prints(),
            section,
            hovered_log_entry,
            header_hovered,
            is_loaded,
            inserted_cartridge: self.inserted_cartridge(),
            launch_options: media_options
                .and_then(|media| launch::game_settings(self, sha1, media)),
        })
    }

    /// Navigate to the detail screen for a game, loading activity in background.
    pub(in crate::app) fn go_to_detail(&mut self, sha1: &str) -> Task<Message> {
        self.menu_open = false;
        self.store.mark_activity_loading(sha1);
        self.screen = Screen::ViewingGame {
            sha1: sha1.to_string(),
            sub_screen: DetailSubScreen::Detail {
                section: Default::default(),
                hovered_log_entry: None,
                header_hovered: false,
                media_options: launch::media_options(self, sha1),
            },
        };
        self.load_activity_async(sha1)
    }

    /// Kick off a background load of activity detail for a game.
    pub(super) fn load_activity_async(&self, sha1: &str) -> Task<Message> {
        let sha1 = sha1.to_string();
        if let Some(game_dir) = self.store.game_dir(&sha1) {
            let game_dir = game_dir.to_path_buf();
            Task::perform(
                smol::unblock(move || {
                    library::store::GameStore::load_raw_activity(&sha1, &game_dir)
                }),
                Message::ActivityLoaded,
            )
        } else {
            Task::none()
        }
    }

    fn library_view(&self) -> Element<'_, Message> {
        let hovered_game = match &self.screen {
            Screen::Library { hovered_game } => hovered_game.as_deref(),
            _ => None,
        };
        library::view::view(library::view::LibraryView {
            store: &self.store,
            hovered_sha1: hovered_game,
            inserted_cartridge: self.inserted_cartridge(),
            dump_progress: self.cartridge_rw.dump_progress.as_ref(),
            homebrew_enabled: self.homebrew_enabled(),
            sort: self.settings.library_sort,
            layout: self.settings.library_layout,
            search: &self.library_search,
            system_filter: self.library_filter,
        })
    }

    pub(super) fn missing_rom_dirs_bar(&self) -> Option<Element<'static, Message>> {
        let count = self
            .settings
            .rom_directories
            .iter()
            .filter(|dir| !dir.exists())
            .count();
        if count == 0 {
            return None;
        }

        let msg = if count == 1 {
            "1 ROM folder is unavailable".to_string()
        } else {
            format!("{count} ROM folders are unavailable")
        };

        Some(
            mouse_area(
                container(
                    row![
                        icons::m_colored(Icon::Warning, iced::Color::WHITE),
                        iced_text(msg).color(iced::Color::WHITE),
                    ]
                    .spacing(s())
                    .align_y(Center),
                )
                .padding(m())
                .width(Fill)
                .style(|_: &iced::Theme| container::Style {
                    background: Some(iced::Color::from_rgb(0.5, 0.15, 0.15).into()),
                    ..Default::default()
                }),
            )
            .on_press(Message::ShowSettings)
            .interaction(mouse::Interaction::Pointer)
            .into(),
        )
    }
}
