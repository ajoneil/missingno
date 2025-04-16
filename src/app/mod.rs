use std::{fs, path::PathBuf};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task, Theme,
    widget::{column, container},
};

use crate::emulator::{GameBoy, cartridge::Cartridge};
use action_bar::action_bar;
use core::{emoji, fonts, horizontal_rule, sizes::l, text};

mod action_bar;
mod core;
mod debugger;
mod load;

pub fn run(rom_path: Option<PathBuf>) -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .settings(iced::Settings {
            default_font: fonts::default(),
            ..Default::default()
        })
        .theme(App::theme)
        .run_with(|| (App::new(rom_path), Task::none()))
}

struct App {
    game: Game,
}

enum Game {
    Unloaded,
    Loading,
    Loaded(debugger::State),
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),
    Debugger(debugger::Message),

    None,
}

impl App {
    fn new(rom_path: Option<PathBuf>) -> Self {
        match rom_path {
            Some(rom_path) => Self {
                game: Game::Loaded(debugger::State::new(GameBoy::new(Cartridge::new(
                    fs::read(rom_path).unwrap(),
                )))),
            },
            None => Self {
                game: Game::Unloaded,
            },
        }
    }

    fn title(&self) -> String {
        if let Game::Loaded(debugger) = &self.game {
            format!("{} - MissingNo.", debugger.game_boy().cartridge().title())
        } else {
            "MissingNo.".into()
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Load(message) => return load::update(message, &mut self.game),

            Message::Debugger(message) => match &mut self.game {
                Game::Loaded(debugger) => return debugger::update(debugger, message),
                _ => {}
            },

            Message::None => {}
        }

        return Task::none();
    }

    fn view(&self) -> Element<'_, Message> {
        column![
            action_bar(self),
            horizontal_rule(),
            container(self.inner()).center(Fill)
        ]
        .into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.game {
            Game::Loaded(debugger) => debugger::debugger(&debugger),
            _ => column![text::xl("Welcome to MissingNo.!"), emoji::xxl("👾")]
                .align_x(Center)
                .spacing(l())
                .into(),
        }
    }
}
