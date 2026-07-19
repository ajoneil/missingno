//! The disassembly row widgets, shared by the Game Boy's hand-built
//! Instructions pane and the generic Disassembly pane so the two render
//! identically: a breakpoint gutter, a bank-prefixed address, syntax-coloured
//! operands, and the current-instruction highlight. The pane owns the walk and
//! decode; this module owns only how one row looks.

use iced::{
    Background, Border, Color, Element, Length,
    alignment::Vertical,
    widget::text::Span,
    widget::{button, container, rich_text, row, text, tooltip},
};

use crate::app::{
    self,
    debugger::{self, sidebar::tooltip_style},
    ui::{fonts, palette, sizes::s},
};
use missingno_core::inspect::{AddressDisplay, Watch, WatchTerm};
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

/// A bank-prefixed window address when a bank is mapped, plain otherwise.
fn address_text(display: AddressDisplay) -> String {
    match display.bank {
        Some(bank) => format!("{bank:02X}:{:04X}", display.window),
        None => format!("{:04X}", display.window),
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
pub fn data_row(display: AddressDisplay, byte: u8) -> Element<'static, app::Message> {
    row![
        container("").width(Length::Fixed(24.0)),
        text(address_text(display))
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
    display: AddressDisplay,
    byte: u8,
    is_current: bool,
    is_breakpoint: bool,
    is_watched: bool,
) -> Element<'static, app::Message> {
    let the_row = row![
        gutter(display, is_breakpoint, is_watched),
        text(address_text(display))
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
    display: AddressDisplay,
    tokens: &[Token],
    is_current: bool,
    is_breakpoint: bool,
    is_watched: bool,
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
        gutter(display, is_breakpoint, is_watched),
        text(address_text(display))
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

/// The compound `{pc: window, bank-key: bank}` watch a switchable-window row
/// composes, or `None` on a row with no bank to pin. Set and matched from the
/// one place so a gutter click and a row's watch-backed mark cannot drift.
pub fn bank_watch(display: &AddressDisplay) -> Option<Watch> {
    let (key, bank) = display.bank_watch?;
    Some(Watch {
        terms: vec![
            WatchTerm {
                key: "pc".to_owned(),
                address: None,
                value: Some(display.window),
            },
            WatchTerm {
                key: key.to_owned(),
                address: None,
                value: Some(bank as u32),
            },
        ],
    })
}

/// Whether one of the active watches is this row's bank watch — order- and
/// duplicate-insensitive so a watch added elsewhere still marks its row.
pub fn row_watched(active: &[Watch], display: &AddressDisplay) -> bool {
    let Some(target) = bank_watch(display) else {
        return false;
    };
    active.iter().any(|watch| same_terms(watch, &target))
}

fn same_terms(a: &Watch, b: &Watch) -> bool {
    a.terms.len() == b.terms.len() && b.terms.iter().all(|term| a.terms.contains(term))
}

/// The gutter marker. A row that carries a plain breakpoint (a bus address or a
/// fixed-bank window) shows the breakpoint dot, toggling it on click. A
/// switchable-window row instead composes a `{pc, bank}` watch: it shows a
/// watch-tinted dot, toggling that watch on click. A synthetic row with no bank
/// to pin shows a dimmed, unclickable dot with a tooltip.
fn gutter(
    display: AddressDisplay,
    is_breakpoint: bool,
    is_watched: bool,
) -> Element<'static, app::Message> {
    if let Some(address) = display.breakpoint {
        let icon = if is_breakpoint {
            dot(palette::RED, true)
        } else {
            dot(palette::SURFACE2, false)
        };
        return button(icon)
            .style(button::text)
            .on_press(
                if is_breakpoint {
                    debugger::Message::ClearBreakpoint(address)
                } else {
                    debugger::Message::SetBreakpoint(address)
                }
                .into(),
            )
            .into();
    }

    if let Some(watch) = bank_watch(&display) {
        let icon = if is_watched {
            dot(palette::TEAL, true)
        } else {
            dot(palette::SURFACE2, false)
        };
        return button(icon)
            .style(button::text)
            .on_press(
                if is_watched {
                    debugger::Message::RemoveWatchpoint(watch)
                } else {
                    debugger::Message::AddWatch(watch)
                }
                .into(),
            )
            .into();
    }

    tooltip(
        dot(palette::SURFACE2, false),
        container(
            text("no breakpoint — bank-shared window")
                .font(fonts::monospace())
                .size(11.0),
        )
        .padding([2.0, s()]),
        tooltip::Position::Right,
    )
    .style(tooltip_style)
    .into()
}

/// The 8px gutter marker: a filled disc (`filled`) or a hollow ring outlined in
/// `color`.
fn dot(color: Color, filled: bool) -> Element<'static, app::Message> {
    container("")
        .width(Length::Fixed(8.0))
        .height(Length::Fixed(8.0))
        .style(move |_: &iced::Theme| {
            if filled {
                container::Style {
                    background: Some(Background::Color(color)),
                    border: Border::default().rounded(4.0),
                    ..Default::default()
                }
            } else {
                container::Style {
                    border: Border::default().rounded(4.0).width(1.0).color(color),
                    ..Default::default()
                }
            }
        })
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

#[cfg(test)]
mod tests {
    use super::{bank_watch, row_watched};
    use missingno_core::inspect::{AddressDisplay, Watch, WatchTerm};

    fn term(key: &str, value: u32) -> WatchTerm {
        WatchTerm {
            key: key.to_owned(),
            address: None,
            value: Some(value),
        }
    }

    #[test]
    fn bank_watch_composes_pc_and_bank_terms() {
        let display = AddressDisplay::banked(0x4123, 3, "rom-bank");
        let watch = bank_watch(&display).expect("a switchable row composes a watch");
        assert_eq!(watch.terms, vec![term("pc", 0x4123), term("rom-bank", 3)]);

        // A plain bus row (or a fixed-bank row) has no bank watch.
        assert!(bank_watch(&AddressDisplay::bus(0x0150, None)).is_none());
        assert!(bank_watch(&AddressDisplay::fixed(0x0150, 0)).is_none());
    }

    #[test]
    fn row_watched_matches_its_compound_order_insensitively() {
        let display = AddressDisplay::banked(0x4123, 3, "rom-bank");
        // Same terms, reversed order — still this row's watch.
        let active = vec![Watch {
            terms: vec![term("rom-bank", 3), term("pc", 0x4123)],
        }];
        assert!(row_watched(&active, &display));

        // Wrong bank, wrong window, and an unrelated watch do not mark the row.
        let others = vec![
            Watch {
                terms: vec![term("pc", 0x4123), term("rom-bank", 2)],
            },
            Watch {
                terms: vec![term("pc", 0x4000), term("rom-bank", 3)],
            },
            Watch::single("bus-read", Some(0x4123), None),
        ];
        assert!(!row_watched(&others, &display));
        assert!(!row_watched(&[], &display));
    }
}
