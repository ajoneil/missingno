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
        screen::ScreenView,
    },
    game_boy::{
        sgb::SgbRenderData,
        video::{palette::PaletteChoice, screen::Screen},
    },
};

pub struct ScreenPane {
    screen_view: ScreenView,
}

#[derive(Debug, Copy, Clone)]
pub enum Message {
    Update(Option<Screen>, Option<SgbRenderData>),
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
            Message::Update(screen, sgb_render_data) => {
                if let Some(screen) = screen {
                    self.screen_view.screen = screen;
                }
                self.screen_view.sgb_render_data = sgb_render_data;
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
