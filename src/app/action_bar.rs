use iced::{
    Element,
    widget::{Button, row},
};

use crate::app::{
    App, Game, Message,
    core::{
        buttons,
        sizes::{m, s, xl},
    },
    debugger, load,
};

pub fn action_bar(app: &App) -> Element<'static, Message> {
    match app.game {
        Game::Unloaded | Game::Loading => row![load(&app.game)],
        Game::Loaded(_) => row![load(&app.game), controls(false)],
    }
    .spacing(xl())
    .padding(m())
    .into()
}

fn load(game: &Game) -> Button<'static, Message> {
    let button = buttons::standard("Load ROM...");
    match game {
        Game::Loading => button,
        _ => button.on_press(load::Message::Pick.into()),
    }
}

fn controls(running: bool) -> Element<'static, Message> {
    row![play_pause(running), step_frame(), reset()]
        .spacing(s())
        .wrap()
        .into()
}

fn play_pause(running: bool) -> Button<'static, Message> {
    if running {
        buttons::success("Pause").on_press(debugger::Message::Pause.into())
    } else {
        buttons::success("Play").on_press(debugger::Message::Run.into())
    }
}

fn step_frame() -> Button<'static, Message> {
    buttons::standard("Step Frame").on_press(debugger::Message::StepFrame.into())
}

fn reset() -> Button<'static, Message> {
    buttons::danger("Reset").on_press(debugger::Message::Reset.into())
}
