use std::{fs, path::PathBuf};

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Subscription, Task, Theme,
    widget::{column, container, svg},
};

use crate::emulator::{GameBoy, cartridge::Cartridge};
use action_bar::ActionBar;
use core::{
    fonts, horizontal_rule,
    icons::{self, Icon},
    sizes::l,
    text,
};

mod action_bar;
mod core;
mod debugger;
mod load;
mod screen;

pub fn run(rom_path: Option<PathBuf>) -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .settings(iced::Settings {
            default_font: fonts::default(),
            ..Default::default()
        })
        .theme(App::theme)
        .run_with(|| (App::new(rom_path), Task::none()))
}

struct App {
    game: Game,
    action_bar: ActionBar,
}

enum Game {
    Unloaded,
    Loading,
    Loaded(debugger::Debugger),
}

#[derive(Debug, Clone)]
enum Message {
    Load(load::Message),
    Debugger(debugger::Message),
    ActionBar(action_bar::Message),

    None,
}

impl App {
    fn new(rom_path: Option<PathBuf>) -> Self {
        let game = match rom_path {
            Some(rom_path) => Game::Loaded(debugger::Debugger::new(GameBoy::new(Cartridge::new(
                fs::read(rom_path).unwrap(),
            )))),

            None => Game::Unloaded,
        };

        Self {
            game,
            action_bar: ActionBar::new(),
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
                Game::Loaded(debugger) => return debugger.update(message),
                _ => {}
            },

            Message::ActionBar(message) => return self.action_bar.update(message),

            Message::None => {}
        }

        return Task::none();
    }

    fn view(&self) -> Element<'_, Message> {
        column![
            self.action_bar.view(self),
            horizontal_rule(),
            container(self.inner()).center(Fill)
        ]
        .into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.game {
            Game::Loaded(debugger) => debugger.view(),
            _ => column![
                text::xl("Welcome to MissingNo.!"),
                icons::xl(Icon::GameBoy)
                    .width(200)
                    .height(200)
                    .style(|theme, _| {
                        svg::Style {
                            color: Some(theme.extended_palette().success.strong.color),
                        }
                    })
            ]
            .align_x(Center)
            .spacing(l())
            .into(),
        }
    }
    pub fn subscription(&self) -> Subscription<Message> {
        if let Game::Loaded(debugger) = &self.game {
            debugger.subscription()
        } else {
            Subscription::none()
        }
    }
}
