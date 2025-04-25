use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task,
    widget::{Button, Column, container, row, toggler},
};
use iced_aw::DropDown;

use crate::app::{
    self, App, Game, LoadedGame,
    core::{
        buttons::{self, icon_label},
        icons::Icon,
        sizes::{m, s, xl},
        text,
    },
    debugger::{
        self,
        panes::{self, DebuggerPane},
    },
    load,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPaneDropdown,
    DismissPaneDropdown,
    ShowPane(DebuggerPane),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::ActionBar(self)
    }
}

pub struct ActionBar {
    pane_dropdown_shown: bool,
}

impl ActionBar {
    pub fn new() -> Self {
        Self {
            pane_dropdown_shown: false,
        }
    }

    pub fn view(&self, app: &App) -> Element<'_, app::Message> {
        match &app.game {
            Game::Unloaded | Game::Loading => row![load(&app.game), self.settings(app)],
            Game::Loaded(_) => row![
                load(&app.game),
                controls(app.running(), app.debugger_enabled),
                self.settings(app)
            ],
        }
        .spacing(xl())
        .padding(m())
        .into()
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::ShowPaneDropdown => self.pane_dropdown_shown = true,
            Message::DismissPaneDropdown => self.pane_dropdown_shown = false,
            Message::ShowPane(pane) => {
                self.pane_dropdown_shown = false;
                return Task::done(panes::Message::ShowPane(pane).into());
            }
        }

        Task::none()
    }

    fn panes(&self, unshown_panes: &[DebuggerPane]) -> Element<'_, app::Message> {
        if unshown_panes.is_empty() {
            buttons::standard("Add panes").into()
        } else {
            DropDown::new(
                buttons::standard("Add panes").on_press(Message::ShowPaneDropdown.into()),
                self.pane_selection(unshown_panes),
                self.pane_dropdown_shown,
            )
            .width(Fill)
            .on_dismiss(Message::DismissPaneDropdown.into())
            .into()
        }
    }

    fn pane_selection(&self, unshown_panes: &[DebuggerPane]) -> Element<'_, app::Message> {
        container(Column::with_children(unshown_panes.iter().map(|pane| {
            buttons::text(text::m(pane.to_string()))
                .on_press(Message::ShowPane(*pane).into())
                .into()
        })))
        .style(container::bordered_box)
        .into()
    }

    fn settings(&self, app: &App) -> Element<'_, app::Message> {
        let row = match &app.game {
            Game::Loaded(LoadedGame::Debugger(debugger)) => {
                row![self.panes(&debugger.panes().unshown_panes())]
            }
            _ => row![],
        };

        container(
            row.push(
                toggler(app.debugger_enabled)
                    .label("Debugger")
                    .on_toggle(|enable| app::Message::ToggleDebugger(enable))
                    .size(m()),
            )
            .push(
                buttons::standard(icon_label(Icon::Settings, "Settings"))
                    .on_press(app::Message::ShowSettings),
            )
            .spacing(m())
            .align_y(Center),
        )
        .align_right(Fill)
        .align_y(Center)
        .into()
    }
}

fn load(game: &Game) -> Button<'static, app::Message> {
    let button = buttons::standard("Load ROM...");
    match game {
        Game::Loading => button,
        _ => button.on_press(load::Message::Pick.into()),
    }
}

fn controls(running: bool, debugger: bool) -> Element<'static, app::Message> {
    let row = row![play_pause(running)];

    let row = row.push_maybe(if debugger {
        Some(step_frame(running))
    } else {
        None
    });

    row.push(reset()).spacing(s()).wrap().into()
}

fn play_pause(running: bool) -> Button<'static, app::Message> {
    if running {
        buttons::success("Pause").on_press(app::Message::Pause.into())
    } else {
        buttons::success("Play").on_press(app::Message::Run.into())
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
