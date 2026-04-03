use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task,
    widget::{Button, container, mouse_area, pick_list, row},
};

use crate::app::{
    self, App, Game, LoadedGame,
    core::{
        buttons,
        icons::{self, Icon},
        sizes::{m, s, xl},
        text as app_text,
    },
    debugger::{
        self,
        panes::{self, DebuggerPane},
    },
    load,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::ActionBar(self)
    }
}

pub struct ActionBar;

impl ActionBar {
    pub fn new() -> Self {
        Self
    }

    pub fn view(&self, app: &App) -> Element<'_, app::Message> {
        // Title comes from viewing context or running game
        let title = match app.screen {
            app::Screen::Detail => {
                // Show the viewed game's title
                app.viewing_sha1
                    .as_ref()
                    .and_then(|sha1| {
                        app.library_cache
                            .entries
                            .iter()
                            .find(|g| g.entry.sha1 == *sha1)
                    })
                    .map(|g| g.entry.display_title())
                    .unwrap_or_default()
            }
            _ => {
                // Show the running game's title
                app.current_game
                    .as_ref()
                    .map(|g| g.entry.display_title())
                    .unwrap_or_default()
            }
        };

        match app.screen {
            app::Screen::Library => {
                row![iced::widget::Space::new().width(Fill), self.settings(app)]
            }
            app::Screen::Detail => {
                let is_this_game_loaded = app
                    .current_game
                    .as_ref()
                    .zip(app.viewing_sha1.as_ref())
                    .map(|(c, v)| c.entry.sha1 == *v && matches!(app.game, Game::Loaded(_)))
                    .unwrap_or(false);

                let mut r = row![
                    container(
                        row![
                            buttons::subtle(icons::m(Icon::Back))
                                .on_press(app::Message::BackToLibrary),
                            app_text::heading(title).wrapping(iced::widget::text::Wrapping::None),
                        ]
                        .spacing(s())
                        .align_y(Center)
                    )
                    .clip(true)
                    .width(Fill),
                ];

                if is_this_game_loaded {
                    r = r.push(
                        row![
                            buttons::primary("Resume").on_press(app::Message::PlayFromDetail),
                            buttons::danger("Stop").on_press(app::Message::StopGame),
                        ]
                        .spacing(s()),
                    );
                } else {
                    r = r.push(buttons::primary("Play").on_press(app::Message::PlayFromDetail));
                }

                r = r.push(self.settings(app));
                r
            }
            app::Screen::Emulator => {
                let is_debugger = matches!(app.game, Game::Loaded(LoadedGame::Debugger(_)));
                let back_action = if is_debugger {
                    app::Message::ToggleDebugger(false)
                } else {
                    app::Message::BackToDetail
                };

                row![
                    container(
                        row![
                            buttons::subtle(icons::m(Icon::Back)).on_press(back_action),
                            mouse_area(
                                app_text::heading(title)
                                    .wrapping(iced::widget::text::Wrapping::None),
                            )
                            .on_press(app::Message::BackToDetail)
                            .interaction(iced::mouse::Interaction::Pointer),
                        ]
                        .spacing(s())
                        .align_y(Center)
                    )
                    .clip(true)
                    .width(Fill),
                    controls(app.running(), app.debugger_enabled),
                    self.settings(app)
                ]
            }
            app::Screen::Settings => {
                // Settings has its own header, shouldn't reach here
                row![]
            }
        }
        .spacing(xl())
        .padding(m())
        .align_y(Center)
        .into()
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::ShowPane(pane) => {
                return Task::done(panes::Message::ShowPane(pane).into());
            }
        }
    }

    fn panes(&self, unshown_panes: Vec<DebuggerPane>) -> Element<'_, app::Message> {
        pick_list(unshown_panes, None::<DebuggerPane>, |pane| {
            Message::ShowPane(pane).into()
        })
        .placeholder("Add pane...")
        .into()
    }

    fn settings(&self, app: &App) -> Element<'_, app::Message> {
        let mut row = row![];

        // Emulator-specific controls
        if app.screen == app::Screen::Emulator {
            if let Game::Loaded(LoadedGame::Debugger(debugger)) = &app.game {
                row = row.push(self.panes(debugger.panes().unshown_panes()));
            }
            if matches!(app.game, Game::Loaded(LoadedGame::Emulator(_))) {
                row = row.push(
                    buttons::subtle(
                        row![icons::m(Icon::Debug), "Debug"]
                            .spacing(s())
                            .align_y(Center),
                    )
                    .on_press(app::Message::ToggleDebugger(true)),
                );
            }
        }

        if app.screen == app::Screen::Library {
            row = row.push(buttons::subtle("Add Game...").on_press(load::Message::Pick.into()));
        }

        row.push(
            buttons::subtle(
                row![icons::m(Icon::Gear), "Settings"]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::ShowSettings),
        )
        .spacing(m())
        .align_y(Center)
        .into()
    }
}

fn controls(running: bool, debugger: bool) -> Element<'static, app::Message> {
    let row = row![play_pause(running)];

    let row = if debugger {
        row.push(step_frame(running))
    } else {
        row
    };

    row.push(reset())
        .push(buttons::danger("Stop").on_press(app::Message::StopGame))
        .spacing(s())
        .wrap()
        .into()
}

fn play_pause(running: bool) -> Button<'static, app::Message> {
    if running {
        buttons::primary("Pause").on_press(app::Message::Pause.into())
    } else {
        buttons::primary("Play").on_press(app::Message::Run.into())
    }
}

fn step_frame(running: bool) -> Button<'static, app::Message> {
    let button = buttons::standard("Step Frame");

    if running {
        button
    } else {
        button.on_press(debugger::Message::StepFrame.into())
    }
}

fn reset() -> Button<'static, app::Message> {
    buttons::danger("Reset").on_press(app::Message::Reset.into())
}
