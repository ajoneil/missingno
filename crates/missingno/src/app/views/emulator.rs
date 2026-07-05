use iced::{
    Element,
    Length::Fill,
    mouse,
    widget::{container, mouse_area},
};

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
                LoadedGame::Emulator(emulator) => emulator.view(fullscreen),
            },
            _ => text::label("No game loaded").into(),
        }
    }
}
