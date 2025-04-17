use iced::{
    Length::Fill,
    widget::{canvas, pane_grid},
};

use crate::{
    app::{
        Message,
        debugger::panes::{AvailablePanes, pane, title_bar},
    },
    emulator::video::screen::Screen,
};

pub fn screen_pane<'a>(screen: &'a Screen) -> pane_grid::Content<'a, Message> {
    pane(
        title_bar("Screen", Some(AvailablePanes::Screen)),
        canvas(screen).width(Fill).height(Fill).into(),
    )
}
