use iced::{
    Length,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, row, scrollable, text, text_input},
};

use crate::{
    app::{
        self,
        core::{
            fonts,
            icons::{self},
            sizes::s,
        },
        debugger::{
            self,
            panes::{self, DebuggerPane, pane, title_bar},
        },
    },
    debugger::Debugger,
};

pub struct BreakpointsPane {
    breakpoint_input: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    BreakpointInputChanged(String),
    AddBreakpoint,
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(panes::Message::Pane(
            panes::PaneMessage::Breakpoints(self),
        )))
    }
}

impl BreakpointsPane {
    pub fn new() -> Self {
        Self {
            breakpoint_input: String::new(),
        }
    }

    pub fn update(&mut self, message: &Message, debugger: &mut Debugger) {
        match message {
            Message::BreakpointInputChanged(input) => {
                self.breakpoint_input = input
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .take(4)
                    .collect()
            }
            Message::AddBreakpoint => {
                if self.breakpoint_input.len() == 4 {
                    debugger
                        .set_breakpoint(u16::from_str_radix(&self.breakpoint_input, 16).unwrap());
                }
            }
        }
    }

    pub fn content(&self, debugger: &Debugger) -> pane_grid::Content<'_, app::Message> {
        pane(
            title_bar("Breakpoints", DebuggerPane::Breakpoints),
            column![self.breakpoints(debugger), self.add_breakpoint()].into(),
        )
    }

    fn breakpoints(&self, debugger: &Debugger) -> iced::Element<'_, app::Message> {
        container(
            scrollable(Column::from_iter(
                debugger
                    .breakpoints()
                    .iter()
                    .map(|address| self.breakpoint(*address)),
            ))
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .style(container::bordered_box)
        .padding(s())
        .into()
    }

    fn breakpoint(&self, address: u16) -> iced::Element<'_, app::Message> {
        container(
            row![
                button(icons::breakpoint_enabled())
                    .on_press(app::debugger::Message::ClearBreakpoint(address).into())
                    .style(button::text),
                text(format!("{:04x}", address)).font(fonts::monospace())
            ]
            .align_y(Vertical::Center),
        )
        .into()
    }

    fn add_breakpoint(&self) -> iced::Element<'_, app::Message> {
        text_input("Add breakpoint...", &self.breakpoint_input)
            .font(fonts::monospace())
            .on_input(|value| Message::BreakpointInputChanged(value).into())
            .on_submit(Message::AddBreakpoint.into())
            .into()
    }
}
