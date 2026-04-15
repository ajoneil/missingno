use std::collections::BTreeSet;

use iced::{
    Background, Border, Element, Length,
    alignment::Vertical,
    widget::text::Span,
    widget::{Column, button, container, pane_grid, rich_text, row, text},
};

use crate::app::{
    self,
    debugger::{
        self,
        panes::{pane, title_bar, title_bar_with_detail},
    },
    ui::{fonts, palette, sizes::s},
};
use missingno_gb::debugger::instructions::{InstructionsIterator, addresses_before};
use missingno_gb::{GameBoy, cpu::instructions::Instruction};

// Syntax highlighting — mapped from palette colors.
use palette::{
    BLUE as SYN_OPCODE, GREEN as SYN_REGISTER, PEACH as SYN_IMMEDIATE, PURPLE as SYN_MEMORY,
    YELLOW as SYN_CONDITION,
};

/// Fixed height per instruction row so partial rows clip cleanly.
const ROW_HEIGHT: f32 = 20.0;

/// Number of instructions to show before the current PC.
const CONTEXT_BEFORE: usize = 4;
/// Number of instructions to show after (and including) the current PC.
const CONTEXT_AFTER: usize = 80;

pub struct InstructionsPane;

impl InstructionsPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(
        &self,
        memory: &GameBoy,
        pc: u16,
        breakpoints: &BTreeSet<u16>,
    ) -> pane_grid::Content<'_, app::Message> {
        let mut instructions = Vec::new();

        // Instructions before PC (backward sweep)
        let before = addresses_before(pc, CONTEXT_BEFORE, memory);
        for &addr in &before {
            let mut iter = InstructionsIterator::new(addr, memory);
            if let Some(decoded) = Instruction::decode(&mut iter) {
                instructions.push(instruction_row(
                    addr,
                    decoded,
                    false,
                    breakpoints.contains(&addr),
                ));
            }
        }

        // Instructions from PC onwards
        let mut iterator = InstructionsIterator::new(pc, memory);
        for _ in 0..CONTEXT_AFTER {
            if let Some(address) = iterator.address {
                if let Some(decoded) = Instruction::decode(&mut iterator) {
                    instructions.push(instruction_row(
                        address,
                        decoded,
                        address == pc,
                        breakpoints.contains(&address),
                    ));
                } else {
                    break;
                }
            }
        }

        let header = if breakpoints.is_empty() {
            title_bar("Instructions")
        } else {
            let detail = format!("{} bp", breakpoints.len(),);
            title_bar_with_detail(
                "Instructions",
                text(detail)
                    .font(fonts::monospace())
                    .size(11.0)
                    .color(palette::MUTED),
            )
        };

        pane(
            header,
            iced::widget::scrollable(Column::from_vec(instructions).width(Length::Fill))
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new()
                        .width(0)
                        .scroller_width(0),
                ))
                .width(Length::Fill)
                .into(),
        )
    }
}

fn instruction_row(
    address: u16,
    instruction: Instruction,
    is_current: bool,
    is_breakpoint: bool,
) -> Element<'static, app::Message> {
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

    let gutter = button(bp_icon).style(button::text).on_press(
        if is_breakpoint {
            debugger::Message::ClearBreakpoint(address)
        } else {
            debugger::Message::SetBreakpoint(address)
        }
        .into(),
    );

    let the_row = row![
        gutter,
        text(format!("{:04X}", address))
            .font(fonts::monospace())
            .size(13.0)
            .color(palette::OVERLAY0),
        highlighted_instruction(&instruction),
    ]
    .align_y(Vertical::Center)
    .spacing(s())
    .height(Length::Fixed(ROW_HEIGHT));

    if is_current {
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
    } else {
        the_row.into()
    }
}

fn highlighted_instruction(instruction: &Instruction) -> Element<'static, app::Message> {
    let formatted = instruction.to_string();
    let spans = tokenize(&formatted);
    rich_text(spans).font(fonts::monospace()).size(13.0).into()
}

fn tokenize(instruction: &str) -> Vec<Span<'static, &'static str>> {
    let mut spans = Vec::new();
    let parts: Vec<&str> = instruction.splitn(2, ' ').collect();

    // Opcode (first word)
    if let Some(&opcode) = parts.first() {
        spans.push(Span {
            text: opcode.to_owned().into(),
            color: Some(SYN_OPCODE),
            ..Default::default()
        });
    }

    // Operands
    if let Some(&rest) = parts.get(1) {
        spans.push(Span {
            text: " ".to_owned().into(),
            ..Default::default()
        });

        for (i, operand) in rest.split(", ").enumerate() {
            if i > 0 {
                spans.push(Span {
                    text: ", ".to_owned().into(),
                    color: Some(palette::MUTED),
                    ..Default::default()
                });
            }

            let color = classify_operand(operand);
            spans.push(Span {
                text: operand.to_owned().into(),
                color: Some(color),
                ..Default::default()
            });
        }
    }

    spans
}

fn classify_operand(operand: &str) -> iced::Color {
    let op = operand.trim();

    // Memory references: anything with brackets or parentheses
    if op.starts_with('[') || op.starts_with('(') {
        return SYN_MEMORY;
    }

    // Conditions
    if matches!(op, "nz" | "z" | "nc" | "c") {
        // "c" is ambiguous — it's both a register and a condition.
        // In context after jp/call/ret/jr it's a condition; otherwise a register.
        // Since we split by ", " the condition is always the first operand of a
        // conditional jump, so we treat standalone "c" as a condition here.
        // This isn't perfect but works for the common cases.
        return SYN_CONDITION;
    }

    // Registers
    if matches!(
        op,
        "a" | "b" | "d" | "e" | "h" | "l" | "af" | "bc" | "de" | "hl" | "sp"
    ) {
        return SYN_REGISTER;
    }

    // Hex immediates ($xxxx) or decimal numbers
    if op.starts_with('$') || op.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return SYN_IMMEDIATE;
    }

    // Fallback
    palette::TEXT
}
