//! A generic registers pane rendered entirely from the seam's inspection
//! schema. Every family registers it; its rows come from the core's
//! [`RegisterGroup`] list, formatted by each register's [`ValueStyle`], so it
//! works over any core without knowing its hardware.

use iced::widget::{Column, Row, column, container, pane_grid, row, text};
use iced::{Element, Length};

use missingno_core::inspect::{FlagName, Register, RegisterGroup, ValueStyle};

use crate::app;
use crate::app::debugger::panes::{self, DebuggerPane, Pane, PaneContext};
use crate::app::ui::{fonts, palette, sizes::s};

/// Fixed width for the register-name column so values line up.
const NAME_WIDTH: f32 = 40.0;

pub struct RegistersPane;

impl Pane for RegistersPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Registers
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(ctx) = ctx else {
            return panes::running_placeholder("Registers");
        };
        let groups = Column::from_iter(ctx.registers.iter().map(group_section)).spacing(s());
        panes::pane(panes::title_bar("Registers"), groups.into())
    }
}

fn group_section(group: &RegisterGroup) -> Element<'static, app::Message> {
    let heading = mono(group.name.to_string()).color(palette::TEXT);
    let rows = Column::from_iter(group.registers.iter().map(register_row));
    column![heading, rows].spacing(s()).into()
}

fn register_row(register: &Register) -> Element<'static, app::Message> {
    let name = container(mono(register.name.to_string()).color(palette::MUTED))
        .width(Length::Fixed(NAME_WIDTH));
    let value: Element<'static, app::Message> = match register.style {
        ValueStyle::Flags(names) => flags_row(register.value, names),
        _ => mono(format_scalar(register)).color(palette::TEXT).into(),
    };
    row![name, value].spacing(s()).into()
}

fn flags_row(value: u32, names: &[FlagName]) -> Element<'static, app::Message> {
    Row::from_iter(
        flag_cells(value, names)
            .into_iter()
            .map(|(display, active)| {
                let color = if active {
                    palette::TEXT
                } else {
                    palette::SURFACE2
                };
                mono(display).color(color).into()
            }),
    )
    .spacing(2.0)
    .into()
}

fn mono<'a>(content: String) -> iced::widget::Text<'a> {
    text(content).font(fonts::monospace())
}

/// The scalar styles' textual value. Hex carries a `$` prefix, uppercase and
/// zero-padded to its nibble width, matching the debugger's assembly-style hex.
fn format_scalar(register: &Register) -> String {
    match register.style {
        ValueStyle::Hex => {
            let width = (register.bits as usize).div_ceil(4).max(1);
            format!("${:0width$X}", register.value, width = width)
        }
        ValueStyle::Dec => register.value.to_string(),
        ValueStyle::Bool => if register.value != 0 { "true" } else { "false" }.to_string(),
        // Flags render as coloured cells, never through this path.
        ValueStyle::Flags(_) => String::new(),
    }
}

/// One display cell per named flag: the uppercased name when set, a dimmed
/// middle dot when clear — the sidebar's convention.
fn flag_cells(value: u32, names: &[FlagName]) -> Vec<(String, bool)> {
    names
        .iter()
        .map(|flag| {
            let active = value & (1 << flag.bit) != 0;
            let display = if active {
                flag.name.to_uppercase()
            } else {
                "\u{00B7}".to_string()
            };
            (display, active)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(value: u32, bits: u8, style: ValueStyle) -> Register {
        Register {
            name: "r",
            value,
            bits,
            style,
        }
    }

    #[test]
    fn hex_pads_to_nibble_width() {
        assert_eq!(format_scalar(&reg(0x5, 8, ValueStyle::Hex)), "$05");
        assert_eq!(format_scalar(&reg(0xAB, 8, ValueStyle::Hex)), "$AB");
        assert_eq!(format_scalar(&reg(0x1F, 16, ValueStyle::Hex)), "$001F");
        assert_eq!(format_scalar(&reg(0x0, 4, ValueStyle::Hex)), "$0");
    }

    #[test]
    fn dec_and_bool_render_plainly() {
        assert_eq!(format_scalar(&reg(42, 8, ValueStyle::Dec)), "42");
        assert_eq!(format_scalar(&reg(1, 1, ValueStyle::Bool)), "true");
        assert_eq!(format_scalar(&reg(0, 1, ValueStyle::Bool)), "false");
    }

    #[test]
    fn flag_cells_uppercase_active_and_dot_inactive() {
        let names = &[
            FlagName { name: "z", bit: 7 },
            FlagName { name: "n", bit: 6 },
            FlagName { name: "c", bit: 4 },
        ];
        // z and c set, n clear.
        let cells = flag_cells(0b1001_0000, names);
        assert_eq!(
            cells,
            vec![
                ("Z".to_string(), true),
                ("\u{00B7}".to_string(), false),
                ("C".to_string(), true),
            ]
        );
    }
}
