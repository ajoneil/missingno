use iced::{
    Border, Element,
    Length::{self, Fill},
    alignment::Vertical,
    widget::{
        button, column, container, row, rule, scrollable, text, text_input, tooltip, Column,
    },
};

use crate::app::{
    self,
    ui::{
        fonts, icons, palette,
        sizes::s,
    },
    debugger,
};
use missingno_gb::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register16, Register8},
};
use missingno_gb::debugger::Debugger;
use missingno_gb::ppu::types::palette::Palette;

use super::interrupts::interrupts;
use super::ppu::ppu_sidebar;

/// Monospace text size for all register labels and values.
const REG: f32 = 14.0;

const SIDEBAR_WIDTH: f32 = 260.0;

pub struct Sidebar {
    breakpoint_input: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    BreakpointInputChanged(String),
    AddBreakpoint,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Debugger(debugger::Message::Sidebar(message))
    }
}

impl Sidebar {
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
                    self.breakpoint_input.clear();
                }
            }
        }
    }

    pub fn view<'a>(&'a self, debugger: &'a Debugger, pal: &'a Palette) -> Element<'a, app::Message> {
        let cpu = debugger.game_boy().cpu();
        let game_boy = debugger.game_boy();

        container(
            scrollable(
                column![
                    pointers(cpu),
                    row![
                        button("Step").on_press(debugger::Message::Step.into()),
                        button("Step Over").on_press(debugger::Message::StepOver.into()),
                    ]
                    .spacing(s()),
                    rule::horizontal(1),
                    register_a_row(cpu),
                    register_pair_row(cpu, Register8::B, Register8::C, Register16::Bc),
                    register_pair_row(cpu, Register8::D, Register8::E, Register16::De),
                    register_pair_row(cpu, Register8::H, Register8::L, Register16::Hl),
                    rule::horizontal(1),
                    interrupts(game_boy),
                    rule::horizontal(1),
                    ppu_sidebar(game_boy.ppu(), pal),
                    rule::horizontal(1),
                    self.breakpoints_view(debugger),
                    self.add_breakpoint(),
                ]
                .spacing(s())
                .padding(s()),
            ),
        )
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .height(Fill)
        .style(sidebar_style)
        .into()
    }

    fn breakpoints_view(&self, debugger: &Debugger) -> Element<'_, app::Message> {
        Column::from_iter(
            debugger
                .breakpoints()
                .iter()
                .map(|address| breakpoint(*address)),
        )
        .into()
    }

    fn add_breakpoint(&self) -> Element<'_, app::Message> {
        text_input("Add breakpoint...", &self.breakpoint_input)
            .font(fonts::monospace())
            .on_input(|value| Message::BreakpointInputChanged(value).into())
            .on_submit(Message::AddBreakpoint.into())
            .into()
    }
}

fn sidebar_style(theme: &iced::Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        ..Default::default()
    }
}

pub fn tooltip_style(theme: &iced::Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(palette.background.strong.color),
        ..Default::default()
    }
}

fn breakpoint(address: u16) -> Element<'static, app::Message> {
    container(
        row![
            button(icons::breakpoint_enabled())
                .on_press(debugger::Message::ClearBreakpoint(address).into())
                .style(button::text),
            text(format!("{:04X}", address)).font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

// --- Pointers + halt ---

fn pointers(cpu: &Cpu) -> Element<'_, app::Message> {
    let halted = cpu.halt_state == HaltState::Halted;
    let pc_color = if halted { palette::OVERLAY0 } else { palette::PURPLE };

    let pc_display: Element<'_, app::Message> = row![
        text("pc")
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(format!("{:04X}", cpu.bus_counter))
            .font(fonts::monospace())
            .size(20.0)
            .color(pc_color),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into();

    let pc_element: Element<'_, app::Message> = if halted {
        tooltip(
            pc_display,
            container(text("halted").font(fonts::monospace()).size(REG))
                .padding([2.0, s()]),
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into()
    } else {
        pc_display
    };

    row![pc_element, pointer("sp", format!("{:04X}", cpu.stack_pointer)),]
        .spacing(s())
        .align_y(Vertical::Center)
        .into()
}

fn pointer(label: &str, value: String) -> Element<'_, app::Message> {
    row![
        text(label)
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(value)
            .font(fonts::monospace())
            .size(20.0)
            .color(palette::PURPLE),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

// --- Registers ---

/// Fixed width for one 8-bit register display ("b 04"), so columns align.
const REG8_WIDTH: f32 = 48.0;

fn register_a_row(cpu: &Cpu) -> Element<'_, app::Message> {
    row![
        container(register8(cpu, Register8::A)).width(Length::Fixed(REG8_WIDTH)),
        container("").width(Length::Fixed(REG8_WIDTH)),
        compound_register(cpu, Register16::Af),
        flags_display(cpu.flags),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn flags_display(flags: Flags) -> Element<'static, app::Message> {
    row![
        flag_char("Z", flags.contains(Flags::ZERO)),
        flag_char("N", flags.contains(Flags::NEGATIVE)),
        flag_char("H", flags.contains(Flags::HALF_CARRY)),
        flag_char("C", flags.contains(Flags::CARRY)),
    ]
    .spacing(2.0)
    .into()
}

fn flag_char(label: &str, set: bool) -> Element<'_, app::Message> {
    let (display, color) = if set {
        (label, palette::TEXT)
    } else {
        ("\u{00B7}", palette::SURFACE2) // middle dot
    };
    text(display)
        .font(fonts::monospace())
        .size(REG)
        .color(color)
        .into()
}

fn register_pair_row(
    cpu: &Cpu,
    reg1: Register8,
    reg2: Register8,
    pair: Register16,
) -> Element<'_, app::Message> {
    row![
        container(register8(cpu, reg1)).width(Length::Fixed(REG8_WIDTH)),
        container(register8(cpu, reg2)).width(Length::Fixed(REG8_WIDTH)),
        compound_register(cpu, pair),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn register8(cpu: &Cpu, register: Register8) -> Element<'_, app::Message> {
    row![
        text(register.to_string())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(format!("{:02X}", cpu.get_register8(register)))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::TEXT),
    ]
    .spacing(s())
    .into()
}

fn compound_register(cpu: &Cpu, register: Register16) -> Element<'_, app::Message> {
    row![
        text(register.to_string())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
        text(format!("{:04X}", cpu.get_register16(register)))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
    ]
    .spacing(s())
    .into()
}
