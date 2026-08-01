use iced::{
    Length::{self, Fill},
    widget::{button, container, pane_grid, responsive, shader, text},
};

use crate::app::{
    self,
    debugger::{
        self,
        panes::{self, DebuggerPane, PaneContext, PaneMessage, pane, title_bar_with_detail},
    },
    ui::fonts,
};
use missingno_core::video::DisplayTechnology;
use missingno_iced::{Frame, PalettePolicy, ScreenView};
use std::sync::Arc;

pub struct ScreenPane {
    screen_view: ScreenView,
    /// Whether the pane simulates the display device (persistence, and later the
    /// pixel grid / scanlines) or shows raw resolved frames — an
    /// inspection-honest instantaneous view. Persisted per-pane in the layout.
    device_simulation: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Update(Arc<Frame>),
    /// Flip this pane between device simulation and raw resolved frames.
    ToggleDeviceSimulation,
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
        let mut pane = Self {
            screen_view: ScreenView::new(),
            device_simulation: true,
        };
        pane.set_device_simulation(true);
        pane
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Update(display) => {
                self.screen_view.apply(&display);
            }
            Message::ToggleDeviceSimulation => {
                self.set_device_simulation(!self.device_simulation);
            }
        }
    }

    fn set_device_simulation(&mut self, on: bool) {
        self.device_simulation = on;
        // Device mode simulates the whole panel/CRT: persistence plus the
        // technology's cosmetic overlay. Raw shows the resolved frame alone.
        self.screen_view.set_persistence(on);
        self.screen_view.set_pixel_grid(on);
        self.screen_view.set_scanlines(on);
    }

    pub fn set_technology(&mut self, technology: DisplayTechnology) {
        self.screen_view.set_technology(technology);
    }

    pub fn set_palette_policy(&mut self, policy: Option<Box<dyn PalettePolicy>>) {
        self.screen_view.set_palette_policy(policy);
    }

    pub fn content(&self, close: pane_grid::Pane) -> pane_grid::Content<'_, app::Message> {
        let mode = if self.device_simulation {
            "Device"
        } else {
            "Raw"
        };
        let toggle = button(text(mode).font(fonts::monospace()).size(11.0))
            .on_press(Message::ToggleDeviceSimulation.into())
            .style(button::text)
            .padding(0);
        pane(
            title_bar_with_detail("Screen", toggle, close),
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

    /// The device/raw mode is persisted through the layout's source slot: raw is
    /// 0, device simulation is 1.
    fn source_index(&self) -> Option<usize> {
        Some(self.device_simulation as usize)
    }

    fn set_source_index(&mut self, index: usize) {
        self.set_device_simulation(index != 0);
    }

    fn set_technology(&mut self, technology: DisplayTechnology) {
        ScreenPane::set_technology(self, technology);
    }

    fn set_palette_policy(&mut self, policy: Option<Box<dyn PalettePolicy>>) {
        ScreenPane::set_palette_policy(self, policy);
    }

    fn screen_view(&self) -> Option<ScreenView> {
        Some(self.screen_view.clone())
    }

    fn adopt_screen_view(&mut self, mut view: ScreenView) {
        // The pane's own device/raw mode wins over whatever the incoming view
        // carried from the other surface.
        view.set_persistence(self.device_simulation);
        view.set_pixel_grid(self.device_simulation);
        view.set_scanlines(self.device_simulation);
        self.screen_view = view;
    }
}
