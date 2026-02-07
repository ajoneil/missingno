use iced::{
    Length::{self, Fill},
    widget::{container, pane_grid, responsive, shader},
};

use crate::{
    app::{
        self,
        debugger::{
            self,
            panes::{self, DebuggerPane, pane, title_bar},
        },
    },
    game_boy::video::screen::Screen,
};

pub struct ScreenPane {
    screen: Screen,
}

#[derive(Debug, Copy, Clone)]
pub enum Message {
    Update(Screen),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(panes::Message::Pane(
            panes::PaneMessage::Screen(self),
        )))
    }
}

impl ScreenPane {
    pub fn new() -> Self {
        Self {
            screen: Screen::new(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Update(screen) => self.screen = screen,
        }
    }

    pub fn content(&self) -> pane_grid::Content<'_, app::Message> {
        pane(
            title_bar("Screen", DebuggerPane::Screen),
            responsive(|size| {
                let shortest = size.width.min(size.height);

                container(
                    shader(&self.screen)
                        .width(Length::Fixed(shortest))
                        .height(Length::Fixed(shortest)),
                )
                .center(Fill)
                .into()
            })
            .into(),
        )
    }
}
