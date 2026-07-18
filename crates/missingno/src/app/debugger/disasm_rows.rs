//! The disassembly row widgets, shared by the Game Boy's hand-built
//! Instructions pane and the generic Disassembly pane so the two render
//! identically: a breakpoint gutter, a bank-prefixed address, syntax-coloured
//! operands, and the current-instruction highlight. The pane owns the walk and
//! decode; this module owns only how one row looks.

use iced::{
    Background, Border, Color, Element, Length,
    alignment::Vertical,
    widget::text::Span,
    widget::{button, container, rich_text, row, text},
};

use crate::app::{
    self,
    debugger::{self},
    ui::{fonts, palette, sizes::s},
};
use missingno_core::isa::{InstructionSet, OperandClass};

// Operand roles mapped to palette colours — the class is semantic, the colour
// is this renderer's choice.
use palette::{
    BLUE as SYN_OPCODE, GREEN as SYN_REGISTER, PEACH as SYN_IMMEDIATE, PURPLE as SYN_MEMORY,
    YELLOW as SYN_CONDITION,
};

/// Fixed height per row so partial rows clip cleanly.
pub const ROW_HEIGHT: f32 = 20.0;

/// One coloured run of a disassembled instruction — the opcode, a separator, or
/// an operand — built once and rendered without re-consulting the instruction
/// set, so a pane can cache the tokens in its readout.
#[derive(Clone)]
pub struct Token {
    text: String,
    color: Option<Color>,
}

/// Split a mnemonic into coloured tokens: the opcode, then each operand tinted
/// by the instruction set's own classification.
pub fn tokenize(isa: &dyn InstructionSet, mnemonic: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut parts = mnemonic.splitn(2, ' ');

    if let Some(opcode) = parts.next() {
        tokens.push(Token {
            text: opcode.to_owned(),
            color: Some(SYN_OPCODE),
        });
    }
    if let Some(rest) = parts.next() {
        tokens.push(Token {
            text: " ".to_owned(),
            color: None,
        });
        for (i, operand) in rest.split(", ").enumerate() {
            if i > 0 {
                tokens.push(Token {
                    text: ", ".to_owned(),
                    color: Some(palette::MUTED),
                });
            }
            tokens.push(Token {
                text: operand.to_owned(),
                color: Some(operand_color(isa.classify_operand(operand))),
            });
        }
    }
    tokens
}

fn operand_color(class: OperandClass) -> Color {
    match class {
        OperandClass::Register => SYN_REGISTER,
        OperandClass::Condition => SYN_CONDITION,
        OperandClass::Immediate => SYN_IMMEDIATE,
        OperandClass::Memory => SYN_MEMORY,
        OperandClass::Plain => palette::TEXT,
    }
}

/// A bank-prefixed address when a bank is mapped, plain otherwise.
fn address_text(address: u32, bank: Option<u16>) -> String {
    match bank {
        Some(bank) => format!("{bank:02X}:{address:04X}"),
        None => format!("{address:04X}"),
    }
}

/// A symbol label row, sitting above the address it names.
pub fn label_row(label: &str) -> Element<'static, app::Message> {
    text(format!("{label}:"))
        .font(fonts::monospace())
        .size(13.0)
        .color(palette::YELLOW)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}

/// A logged data byte within a disassembly — an empty gutter (data can't hold a
/// breakpoint), the address, and its byte as `db $XX`.
pub fn data_row(address: u32, bank: Option<u16>, byte: u8) -> Element<'static, app::Message> {
    row![
        container("").width(Length::Fixed(24.0)),
        text(address_text(address, bank))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::OVERLAY0),
        text(format!("db ${byte:02X}"))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::MUTED),
    ]
    .align_y(Vertical::Center)
    .spacing(s())
    .height(Length::Fixed(ROW_HEIGHT))
    .into()
}

/// A raw byte row for a core with no instruction set: the same chrome as an
/// instruction row — breakpoint gutter and current highlight — carrying `db $XX`.
pub fn byte_row(
    address: u32,
    bank: Option<u16>,
    byte: u8,
    is_current: bool,
    is_breakpoint: bool,
) -> Element<'static, app::Message> {
    let the_row = row![
        gutter(address, is_breakpoint),
        text(address_text(address, bank))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::OVERLAY0),
        text(format!("db ${byte:02X}"))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::MUTED),
    ]
    .align_y(Vertical::Center)
    .spacing(s())
    .height(Length::Fixed(ROW_HEIGHT));
    highlight_if_current(the_row.into(), is_current)
}

/// A decoded instruction row: breakpoint gutter, address, and the coloured
/// tokens, wrapped in the current-instruction highlight when it holds the PC.
pub fn instruction_row(
    address: u32,
    bank: Option<u16>,
    tokens: &[Token],
    is_current: bool,
    is_breakpoint: bool,
) -> Element<'static, app::Message> {
    let spans: Vec<Span<'static, &'static str>> = tokens
        .iter()
        .map(|token| Span {
            text: token.text.clone().into(),
            color: token.color,
            ..Default::default()
        })
        .collect();

    let the_row = row![
        gutter(address, is_breakpoint),
        text(address_text(address, bank))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::OVERLAY0),
        rich_text(spans).font(fonts::monospace()).size(13.0),
    ]
    .align_y(Vertical::Center)
    .spacing(s())
    .height(Length::Fixed(ROW_HEIGHT));
    highlight_if_current(the_row.into(), is_current)
}

/// The breakpoint gutter: a filled dot when set, a hollow ring otherwise,
/// toggling the breakpoint at `address` when clicked.
fn gutter(address: u32, is_breakpoint: bool) -> Element<'static, app::Message> {
    let bp_icon: Element<'static, app::Message> = if is_breakpoint {
        container("")
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(|_: &iced::Theme| container::Style {
                background: Some(Background::Color(palette::RED)),
                border: Border::default().rounded(4.0),
                ..Default::default()
            })
            .into()
    } else {
        container("")
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(|_: &iced::Theme| container::Style {
                border: Border::default()
                    .rounded(4.0)
                    .width(1.0)
                    .color(palette::SURFACE2),
                ..Default::default()
            })
            .into()
    };

    button(bp_icon)
        .style(button::text)
        .on_press(
            if is_breakpoint {
                debugger::Message::ClearBreakpoint(address)
            } else {
                debugger::Message::SetBreakpoint(address)
            }
            .into(),
        )
        .into()
}

/// Wrap a row in the current-instruction highlight — a purple border and wash —
/// or return it unchanged.
fn highlight_if_current(
    the_row: Element<'static, app::Message>,
    is_current: bool,
) -> Element<'static, app::Message> {
    if !is_current {
        return the_row;
    }
    container(the_row)
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(iced::Color::from_rgba(
                0xcb as f32 / 255.0,
                0xa6 as f32 / 255.0,
                0xf7 as f32 / 255.0,
                0.08,
            ))),
            border: Border {
                width: 2.0,
                color: palette::PURPLE,
                ..Border::default()
            },
            ..Default::default()
        })
        .width(Length::Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .into()
}
