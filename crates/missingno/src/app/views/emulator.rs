use iced::{
    Element,
    Length::Fill,
    mouse,
    widget::{container, mouse_area},
};

use crate::app::emulator::{CaptureKind, PlayLogEntry};
use crate::app::library::activity::EventKind;
use crate::app::ui::text;
use crate::app::{App, Game, LoadedGame, Message};

impl App {
    pub(super) fn fullscreen_emulator_view(&self, cursor_hidden: bool) -> Element<'_, Message> {
        let screen = self.emulator_view(true);
        let content = container(screen).center(Fill).style(|_| container::Style {
            background: Some(iced::Color::BLACK.into()),
            ..Default::default()
        });
        let mut area = mouse_area(content).on_move(|_| Message::MouseMoved);
        if cursor_hidden {
            area = area.interaction(mouse::Interaction::Hidden);
        }
        area.into()
    }

    pub(super) fn emulator_view(&self, fullscreen: bool) -> Element<'_, Message> {
        match &self.game {
            Game::Loaded(game) => match game {
                LoadedGame::Debugger(debugger) => debugger.view(),
                LoadedGame::Emulator(emulator) => {
                    let play_log = self.build_play_log();
                    emulator.view(fullscreen, &play_log)
                }
            },
            _ => text::label("No game loaded").into(),
        }
    }

    /// Interleave the live session's screenshots and prints chronologically,
    /// pairing each capture event with its cached thumbnail handle.
    fn build_play_log(&self) -> Vec<PlayLogEntry<'_>> {
        let Some(session) = self.current_game.as_ref().and_then(|g| g.session.as_ref()) else {
            return Vec::new();
        };
        let screenshots = self.store.live_screenshots();
        let prints = self.store.live_prints();
        let (mut si, mut pi) = (0, 0);
        let mut log = Vec::new();
        for (event_index, event) in session.events.iter().enumerate() {
            match &event.kind {
                EventKind::Screenshot { .. } => {
                    if let Some(handle) = screenshots.get(si) {
                        log.push(PlayLogEntry {
                            kind: CaptureKind::Screenshot,
                            handle,
                            at: event.at,
                            event_index,
                        });
                    }
                    si += 1;
                }
                EventKind::Print { .. } => {
                    if let Some(handle) = prints.get(pi) {
                        log.push(PlayLogEntry {
                            kind: CaptureKind::Print,
                            handle,
                            at: event.at,
                            event_index,
                        });
                    }
                    pi += 1;
                }
                _ => {}
            }
        }
        log
    }
}
