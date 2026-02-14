use iced::{
    Length::{self, Fill},
    widget::{container, pane_grid, responsive, shader},
};

use crate::app::{
    self,
    debugger::{
        self,
        panes::{self, DebuggerPane, pane, title_bar},
    },
    screen::{ScreenDisplay, ScreenView},
};
use missingno_core::game_boy::video::palette::PaletteChoice;

pub struct ScreenPane {
    screen_view: ScreenView,
}

#[derive(Debug, Copy, Clone)]
pub enum Message {
    Update(ScreenDisplay),
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
            screen_view: ScreenView::new(),
        }
    }

    pub fn with_screen(screen_view: ScreenView) -> Self {
        Self { screen_view }
    }

    pub fn screen_view(&self) -> &ScreenView {
        &self.screen_view
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Update(display) => {
                self.screen_view.apply(display);
            }
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.screen_view.palette = palette;
    }

    pub fn content(&self) -> pane_grid::Content<'_, app::Message> {
        pane(
            title_bar("Screen", DebuggerPane::Screen),
            responsive(|size| {
                let shortest = size.width.min(size.height);

                container(
                    shader(&self.screen_view)
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
