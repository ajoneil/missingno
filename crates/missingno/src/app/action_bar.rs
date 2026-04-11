use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task,
    widget::{Button, container, mouse_area, pick_list, row},
};

use crate::app::{
    self, App, Game, LoadedGame,
    ui::{
        buttons,
        icons::{self, Icon},
        sizes::{m, s, xl},
        text as app_text,
    },
    debugger::{
        self,
        panes::{self, DebuggerPane},
    },
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
            app::Screen::Detail | app::Screen::ScreenshotGallery => {
                // Show the viewed game's title
                app.viewing_sha1
                    .as_ref()
                    .and_then(|sha1| app.store.entry(sha1))
                    .map(|e| e.display_title())
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
                let mut r = row![app_text::heading("").width(Fill)];
                if app.settings.internet_enabled && app.settings.homebrew_hub_enabled {
                    r = r.push(
                        buttons::subtle(
                            row![icons::m(Icon::Globe), "Browse Homebrew"]
                                .spacing(s())
                                .align_y(Center),
                        )
                        .on_press(app::Message::OpenHomebrewBrowser),
                    );
                }
                r = r.push(self.trailing(app));
                r
            }
            app::Screen::HomebrewBrowser => {
                row![
                    container(
                        row![
                            buttons::subtle(icons::m(Icon::Back)).on_press(
                                app::Message::HomebrewBrowser(
                                    crate::app::library::homebrew_browser::Message::Back,
                                )
                            ),
                            app_text::heading("Homebrew Hub")
                                .wrapping(iced::widget::text::Wrapping::None),
                        ]
                        .spacing(s())
                        .align_y(Center)
                    )
                    .clip(true)
                    .width(Fill),
                    buttons::subtle(
                        row![icons::m(Icon::Globe), "hh.gbdev.io"]
                            .spacing(s())
                            .align_y(Center),
                    )
                    .on_press(app::Message::OpenUrl("https://hh.gbdev.io")),
                    self.trailing(app),
                ]
            }
            app::Screen::ScreenshotGallery => {
                row![
                    container(
                        row![
                            buttons::subtle(icons::m(Icon::Back)).on_press(
                                app::Message::ScreenshotGallery(
                                    crate::app::library::screenshot_gallery::Message::Back,
                                )
                            ),
                            app_text::heading(title).wrapping(iced::widget::text::Wrapping::None),
                        ]
                        .spacing(s())
                        .align_y(Center)
                    )
                    .clip(true)
                    .width(Fill),
                    self.trailing(app),
                ]
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
                    self.trailing(app)
                ]
            }
            // Detail, Settings, CartridgeActions, FlashCartridge manage their
            // own headers and never render through the ActionBar.
            _ => unreachable!(),
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

    fn trailing(&self, app: &App) -> Element<'_, app::Message> {
        let mut row = row![];

        // Debugger pane picker
        if app.screen == app::Screen::Emulator {
            if let Game::Loaded(LoadedGame::Debugger(debugger)) = &app.game {
                row = row.push(self.panes(debugger.panes().unshown_panes()));
            }
        }

        row.push(
            buttons::subtle(icons::m(Icon::Menu))
                .on_press(app::Message::ToggleMenu),
        )
        .spacing(m())
        .align_y(Center)
        .into()
    }
}

fn controls(running: bool, debugger: bool) -> Element<'static, app::Message> {
    let row = row![play_pause(running)];

    let row = if debugger {
        row.push(step_frame(running)).push(capture_frame(running))
    } else {
        row
    };

    row.push(reset())
        .push(stop())
        .spacing(s())
        .wrap()
        .into()
}

fn play_pause(running: bool) -> Button<'static, app::Message> {
    if running {
        buttons::primary("Pause").on_press(app::Message::Pause.into())
    } else {
        buttons::primary(
            row![icons::m(Icon::Play), "Play"]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(app::Message::Run.into())
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

fn capture_frame(running: bool) -> Button<'static, app::Message> {
    let button = buttons::standard("Capture Frame");

    if running {
        button
    } else {
        button.on_press(debugger::Message::CaptureFrame.into())
    }
}

fn reset() -> Button<'static, app::Message> {
    buttons::danger("Reset").on_press(app::Message::Reset.into())
}

fn stop() -> Button<'static, app::Message> {
    buttons::danger("Stop").on_press(app::Message::StopGame)
}
