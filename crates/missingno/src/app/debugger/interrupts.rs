use iced::{
    Background, Border, Color, Element,
    Length,
    alignment::Vertical,
    widget::{column, container, row, text, tooltip, Space},
};

use crate::app::{
    Message,
    ui::{
        fonts,
        icons::{self, Icon},
        palette,
        sizes::{border_s, s},
    },
    debugger::sidebar::tooltip_style,
};
use missingno_gb::GameBoy;
use missingno_gb::interrupts::Interrupt;

const LABEL_SIZE: f32 = 14.0;

/// Fixed width for each icon/pip column so they align across rows.
const COL: f32 = 24.0;

/// Fixed width for the label column — sized to fit the IME badge so it sits flush left.
const LABEL_COL: f32 = 36.0;

pub fn interrupts(game_boy: &GameBoy) -> Element<'static, Message> {
    let ints = game_boy.interrupts();
    let ime = game_boy.cpu().interrupts_enabled();

    column![
        // Icon header row
        row![
            container(ime_badge(ime))
                .width(Length::Fixed(LABEL_COL))
                .align_right(Length::Fixed(LABEL_COL)),
            icon_cell(Icon::Monitor, "VBlank"),
            icon_cell(Icon::Eye, "Stat"),
            icon_cell(Icon::Clock, "Timer"),
            icon_cell(Icon::Wifi, "Serial"),
            icon_cell(Icon::Gamepad, "Joypad"),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
        // IE row
        row![
            container(text("IE").font(fonts::monospace()).size(LABEL_SIZE).color(palette::MUTED))
                .width(Length::Fixed(LABEL_COL))
                .align_right(Length::Fixed(LABEL_COL)),
            pip_cell(ints.enabled(Interrupt::VideoBetweenFrames), palette::GREEN),
            pip_cell(ints.enabled(Interrupt::VideoStatus), palette::GREEN),
            pip_cell(ints.enabled(Interrupt::Timer), palette::GREEN),
            pip_cell(ints.enabled(Interrupt::Serial), palette::GREEN),
            pip_cell(ints.enabled(Interrupt::Joypad), palette::GREEN),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
        // IF row
        row![
            container(text("IF").font(fonts::monospace()).size(LABEL_SIZE).color(palette::MUTED))
                .width(Length::Fixed(LABEL_COL))
                .align_right(Length::Fixed(LABEL_COL)),
            pip_cell(ints.requested(Interrupt::VideoBetweenFrames), palette::YELLOW),
            pip_cell(ints.requested(Interrupt::VideoStatus), palette::YELLOW),
            pip_cell(ints.requested(Interrupt::Timer), palette::YELLOW),
            pip_cell(ints.requested(Interrupt::Serial), palette::YELLOW),
            pip_cell(ints.requested(Interrupt::Joypad), palette::YELLOW),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
    ]
    .spacing(s())
    .into()
}

fn icon_cell(icon: Icon, name: &str) -> Element<'static, Message> {
    tooltip(
        container(icons::m_muted(icon))
            .width(Length::Fixed(COL))
            .center_x(COL),
        container(text(name.to_owned()).font(fonts::monospace()).size(LABEL_SIZE))
            .padding([2.0, s()]),
        tooltip::Position::Top,
    )
    .style(tooltip_style)
    .into()
}

fn pip_cell(active: bool, active_color: Color) -> Element<'static, Message> {
    container(pip(active, active_color))
        .width(Length::Fixed(COL))
        .center_x(COL)
        .center_y(LABEL_SIZE)
        .into()
}

pub fn pip(active: bool, active_color: Color) -> Element<'static, Message> {
    let (bg, border_color) = if active {
        (Some(Background::Color(active_color)), active_color)
    } else {
        (None, palette::SURFACE2)
    };

    container(Space::new())
        .width(10.0)
        .height(10.0)
        .style(move |_: &iced::Theme| container::Style {
            background: bg,
            border: Border::default().rounded(5.0).width(1.5).color(border_color),
            ..Default::default()
        })
        .into()
}

fn ime_badge(enabled: bool) -> Element<'static, Message> {
    let text_color = if enabled {
        palette::GREEN
    } else {
        palette::SURFACE2
    };
    let bg = if enabled {
        Some(Background::Color(Color::from_rgba(
            0xa6 as f32 / 255.0,
            0xe3 as f32 / 255.0,
            0xa1 as f32 / 255.0,
            0.12,
        )))
    } else {
        None
    };

    container(
        text("IME")
            .font(fonts::monospace())
            .size(LABEL_SIZE)
            .color(text_color),
    )
    .padding([2.0, 4.0])
    .style(move |_: &iced::Theme| container::Style {
        background: bg,
        border: Border::default().rounded(border_s()),
        ..Default::default()
    })
    .into()
}
