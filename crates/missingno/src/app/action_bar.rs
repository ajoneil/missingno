use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    widget::{Button, container, mouse_area, row},
};

use crate::app::{
    self, App, Game, LoadedGame, automation, debugger,
    ui::{
        buttons,
        icons::{self, Icon},
        labelled::labelled,
        sizes::{m, s, xl},
        text as app_text,
    },
};

pub struct ActionBar;

impl ActionBar {
    pub fn new() -> Self {
        Self
    }

    pub fn view(&self, app: &App) -> Element<'_, app::Message> {
        // Title comes from viewing context or running game
        let title = match &app.screen {
            app::Screen::ViewingGame { sha1, .. } => {
                // Show the viewed game's title
                app.store
                    .entry(sha1)
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

        match &app.screen {
            app::Screen::Library { .. } => {
                let mut r = row![app_text::heading("").width(Fill)];
                if app.settings.internet_enabled && app.settings.homebrew_hub_enabled {
                    r = r.push(automation::tag(
                        automation::ids::ACTION_BAR_HOMEBREW,
                        buttons::subtle(
                            row![icons::m(Icon::Globe), "Browse Homebrew"]
                                .spacing(s())
                                .align_y(Center),
                        )
                        .on_press(app::Message::OpenHomebrewBrowser),
                    ));
                }
                r = r.push(self.trailing());
                r
            }
            app::Screen::HomebrewBrowser { .. } => {
                row![
                    container(
                        row![
                            automation::tag(
                                automation::ids::ACTION_BAR_BACK,
                                labelled(
                                    buttons::subtle(icons::m(Icon::Back)).on_press(
                                        app::Message::HomebrewBrowser(
                                            crate::app::library::homebrew_browser::Message::Back,
                                        )
                                    ),
                                    "Back to library",
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
                    self.trailing(),
                ]
            }
            app::Screen::ViewingGame {
                sub_screen: app::DetailSubScreen::ScreenshotGallery { .. },
                ..
            } => {
                row![
                    container(
                        row![
                            automation::tag(
                                automation::ids::ACTION_BAR_BACK,
                                labelled(
                                    buttons::subtle(icons::m(Icon::Back)).on_press(
                                        app::Message::ScreenshotGallery(
                                            crate::app::library::screenshot_gallery::Message::Back,
                                        )
                                    ),
                                    "Back to game",
                                )
                            ),
                            app_text::heading(title).wrapping(iced::widget::text::Wrapping::None),
                        ]
                        .spacing(s())
                        .align_y(Center)
                    )
                    .clip(true)
                    .width(Fill),
                    self.trailing(),
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
                            automation::tag(
                                automation::ids::EMULATOR_BACK,
                                labelled(
                                    buttons::subtle(icons::m(Icon::Back)).on_press(back_action),
                                    if is_debugger {
                                        "Close debugger"
                                    } else {
                                        "Back to game details"
                                    },
                                )
                            ),
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
                    controls(app.running(), is_debugger),
                    self.trailing()
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

    fn trailing(&self) -> Element<'_, app::Message> {
        let row = row![];

        row.push(automation::tag(
            automation::ids::ACTION_BAR_MENU,
            labelled(
                buttons::subtle(icons::m(Icon::Menu)).on_press(app::Message::ToggleMenu),
                "Open menu",
            ),
        ))
        .spacing(m())
        .align_y(Center)
        .into()
    }
}

fn controls(running: bool, debugger: bool) -> Element<'static, app::Message> {
    let mut r = row![];

    if debugger {
        r = r
            .push(automation::tag(
                automation::ids::EMULATOR_STEP,
                step(running),
            ))
            .push(automation::tag(
                automation::ids::EMULATOR_STEP_OVER,
                step_over(running),
            ));
    }

    r.push(automation::tag(
        automation::ids::EMULATOR_PLAY_PAUSE,
        play_pause(running),
    ))
    .spacing(s())
    .wrap()
    .into()
}

fn play_pause(running: bool) -> Button<'static, app::Message> {
    if running {
        buttons::primary("Pause").on_press(app::Message::Pause)
    } else {
        buttons::primary(
            row![icons::m(Icon::Play), "Play"]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(app::Message::Run)
    }
}

fn step(running: bool) -> Button<'static, app::Message> {
    let button = buttons::standard("Step");
    if running {
        button
    } else {
        button.on_press(debugger::Message::Step.into())
    }
}

fn step_over(running: bool) -> Button<'static, app::Message> {
    let button = buttons::standard("Over");
    if running {
        button
    } else {
        button.on_press(debugger::Message::StepOver.into())
    }
}
