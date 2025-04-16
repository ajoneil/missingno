use iced::{
    Element,
    widget::{Button, row},
};

use crate::app::{
    App, Game, Message,
    core::{buttons, sizes::m},
    load,
};

pub fn action_bar(app: &App) -> Element<'static, Message> {
    row![load(&app.game)].padding(m()).into()
}

pub fn load(game: &Game) -> Button<'static, Message> {
    let button = buttons::success("Load ROM...");
    match game {
        Game::Loading => button,
        _ => button.on_press(load::Message::Pick.into()),
    }
}
