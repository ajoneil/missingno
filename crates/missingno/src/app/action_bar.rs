use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task,
    widget::{Button, container, pick_list, row, toggler},
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
use missingno_gb::ppu::types::palette::PaletteChoice;

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
    SelectPalette(PaletteChoice),
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
        match &app.game {
            Game::Unloaded | Game::Loading => {
                row![
                    container(app_text::xl("Library")).width(Fill),
                    self.settings(app)
                ]
            }
            Game::Loaded(_) => {
                let title = app
                    .current_game
                    .as_ref()
                    .map(|g| g.entry.title.clone())
                    .unwrap_or_default();

                row![
                    buttons::subtle(
                        row![icons::m(Icon::Back), "Library"]
                            .spacing(s())
                            .align_y(Center),
                    )
                    .on_press(app::Message::BackToLibrary),
                    container(
                        app_text::xl(title).wrapping(iced::widget::text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Fill),
                    controls(app.running(), app.debugger_enabled),
                    self.settings(app)
                ]
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
            Message::SelectPalette(palette) => {
                return Task::done(app::Message::SelectPalette(palette));
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

    fn palette_selector(&self, current: PaletteChoice) -> Element<'_, app::Message> {
        pick_list(PaletteChoice::ALL, Some(current), |choice| {
            Message::SelectPalette(choice).into()
        })
        .into()
    }

    fn settings(&self, app: &App) -> Element<'_, app::Message> {
        let mut row = match &app.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                row![self.panes(debugger.panes().unshown_panes())]
            }
            _ => row![],
        };

        if matches!(app.game, Game::Loaded(_)) {
            if !app.sgb_active() {
                row = row.push(self.palette_selector(app.settings.palette));
            }
            row = row.push(
                toggler(app.debugger_enabled)
                    .label("Debugger")
                    .on_toggle(|enable| app::Message::ToggleDebugger(enable))
                    .size(m()),
            );
        }

        if !matches!(app.game, Game::Loaded(_)) {
            row = row.push(
                buttons::subtle("Add Game...")
                    .on_press(load::Message::Pick.into()),
            );
        }

        row.push(
            buttons::subtle(row![icons::m(Icon::Gear), "Settings"].spacing(s()).align_y(Center))
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

    row.push(reset()).spacing(s()).wrap().into()
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
