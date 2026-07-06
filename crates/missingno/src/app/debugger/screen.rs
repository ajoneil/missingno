use iced::{
    Length::{self, Fill},
    widget::{container, pane_grid, responsive, shader},
};

use crate::app::{
    self,
    debugger::{
        self,
        panes::{self, DebuggerPane, PaneContext, PaneMessage, pane, title_bar},
    },
    screen::{ScreenDisplay, ScreenView},
};
use missingno_gb::ppu::types::palette::PaletteChoice;

pub struct ScreenPane {
    screen_view: ScreenView,
}

#[derive(Debug, Clone)]
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

    pub fn set_frame_blending(&mut self, blend: bool) {
        self.screen_view.blend = blend;
    }

    pub fn content(&self) -> pane_grid::Content<'_, app::Message> {
        pane(
            title_bar("Screen"),
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

impl panes::Pane for ScreenPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Screen
    }

    /// The screen renders its own live frame slot even without a context.
    fn view<'a>(&'a self, _ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        self.content()
    }

    fn on_message(&mut self, message: &PaneMessage) {
        if let PaneMessage::Screen(message) = message {
            self.update(message.clone());
        }
    }

    fn set_palette(&mut self, palette: PaletteChoice) {
        ScreenPane::set_palette(self, palette);
    }

    fn set_frame_blending(&mut self, blend: bool) {
        ScreenPane::set_frame_blending(self, blend);
    }

    fn screen_view(&self) -> Option<ScreenView> {
        Some(self.screen_view.clone())
    }

    fn adopt_screen_view(&mut self, view: ScreenView) {
        self.screen_view = view;
    }
}
