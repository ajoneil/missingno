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
    screen::{Frame, ScreenView},
};
use missingno_gb::ppu::types::palette::PaletteChoice;
use std::sync::Arc;

pub struct ScreenPane {
    screen_view: ScreenView,
}

#[derive(Debug, Clone)]
pub enum Message {
    Update(Arc<Frame>),
}

impl From<Message> for app::Message {
    fn from(val: Message) -> Self {
        app::Message::Debugger(debugger::Message::Pane(panes::Message::Broadcast(
            panes::PaneMessage::Screen(val),
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
                self.screen_view.apply(&display);
            }
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.screen_view.palette = palette;
    }

    pub fn set_frame_blending(&mut self, blend: bool) {
        self.screen_view.blend = blend;
    }

    pub fn content(&self, close: pane_grid::Pane) -> pane_grid::Content<'_, app::Message> {
        pane(
            title_bar("Screen", close),
            responsive(|size| {
                let (width, height) = self.screen_view.fitted_size(size);

                container(
                    shader(&self.screen_view)
                        .width(Length::Fixed(width))
                        .height(Length::Fixed(height)),
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
    fn view<'a>(
        &'a self,
        _ctx: Option<&PaneContext<'_>>,
        id: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        self.content(id)
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
