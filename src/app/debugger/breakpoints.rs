use iced::{
    Length,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, row, scrollable, text, text_input},
};

use crate::{
    app::{
        self,
        core::{fonts, sizes::s},
        debugger::panes::{pane, title_bar},
    },
    debugger::Debugger,
};

pub struct State {
    breakpoint_input: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    BreakpointInputChanged(String),
    AddBreakpoint,
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(app::debugger::Message::BreakpointPane(self))
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            breakpoint_input: String::new(),
        }
    }

    pub fn update(&mut self, message: Message, debugger: &mut Debugger) {
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
}

pub fn breakpoints_pane<'a>(
    debugger: &'a Debugger,
    state: &'a State,
) -> pane_grid::Content<'a, app::Message> {
    pane(
        title_bar("Breakpoints"),
        column![breakpoints(debugger), add_breakpoint(state)].into(),
    )
}

fn breakpoints(debugger: &Debugger) -> iced::Element<'_, app::Message> {
    container(
        scrollable(Column::from_iter(
            debugger
                .breakpoints()
                .iter()
                .map(|address| breakpoint(*address)),
        ))
        .height(Length::Fill)
        .width(Length::Fill),
    )
    .style(container::bordered_box)
    .padding(s())
    .into()
}

fn breakpoint(address: u16) -> iced::Element<'static, app::Message> {
    container(
        row![
            button(text("🔴").font(fonts::emoji()))
                .on_press(app::debugger::Message::ClearBreakpoint(address).into())
                .style(button::text),
            text(format!("{:04x}", address)).font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

fn add_breakpoint(state: &State) -> iced::Element<'_, app::Message> {
    text_input("Add breakpoint...", &state.breakpoint_input)
        .font(fonts::monospace())
        .on_input(|value| Message::BreakpointInputChanged(value).into())
        .on_submit(Message::AddBreakpoint.into())
        .into()
}
