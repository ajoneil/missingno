use iced::{
    Background, Border, Color, Element,
    Length,
    alignment::Vertical,
    widget::{column, container, pane_grid, row, rule, scrollable, text, Space},
};

use crate::app::{
    Message,
    screen::iced_color,
    ui::{
        fonts, palette,
        sizes::{s, xs},
    },
    debugger::{
        interrupts::pip,
        panes::{pane, pip_title_bar_with_detail},
    },
};
use missingno_gb::ppu::{
    Ppu,
    rendering::Mode,
    types::{
        control::Control,
        palette::{Palette, PaletteMap},
        tiles::TileAddressMode,
    },
};

pub mod palette_widget;
pub mod sprites;
mod tile_atlas;
pub mod tile_maps;
mod tile_widget;
pub mod tiles;

/// Monospace text size for register labels and values.
const REG: f32 = 14.0;
/// Small label size for section headers and dim annotations.
const LABEL: f32 = 11.0;

pub struct PpuPane;

impl PpuPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, ppu: &Ppu, pal: &Palette) -> pane_grid::Content<'_, Message> {
        let control = ppu.control();
        let mode = ppu.mode();
        let palettes = ppu.palettes();

        pane(
            pip_title_bar_with_detail("PPU", control.video_enabled(), mode_label(mode)),
            scrollable(
                column![
                    row![
                        timing_value("ly", ppu.video.ly()),
                        timing_value("lx", ppu.lx()),
                    ]
                    .spacing(s())
                    .align_y(Vertical::Center),
                    rule::horizontal(1),
                    background_section(control, palettes.background.output(), ppu, pal),
                    rule::horizontal(1),
                    window_section(control, ppu),
                    rule::horizontal(1),
                    sprites_section(control, palettes, pal),
                ]
                .spacing(s())
                .padding(s()),
            )
            .into(),
        )
    }
}

// --- Mode label ---

fn mode_label(mode: Mode) -> Element<'static, Message> {
    let (label, color) = match mode {
        Mode::HorizontalBlank => ("HBlank", palette::BLUE),
        Mode::VerticalBlank => ("VBlank", palette::GREEN),
        Mode::OamScan => ("OAM Scan", palette::YELLOW),
        Mode::Drawing => ("Drawing", palette::PEACH),
    };

    text(label)
        .font(fonts::monospace())
        .size(LABEL)
        .color(color)
        .into()
}

// --- Timing values ---

fn timing_value(label: &str, value: impl std::fmt::Display) -> Element<'static, Message> {
    label_value(label, &value.to_string())
}

// --- Subsystem sections ---

fn background_section(
    control: Control,
    bgp: u8,
    ppu: &Ppu,
    pal: &Palette,
) -> Element<'static, Message> {
    let scx = ppu.read_register(missingno_gb::ppu::Register::BackgroundViewportX);
    let scy = ppu.read_register(missingno_gb::ppu::Register::BackgroundViewportY);

    column![
        row![
            enable_pip("bg", control.background_and_window_enabled()),
            label_value("map", tile_map_addr(control.background_tile_map().0)),
            label_value("tile", tile_addr(control.tile_address_mode())),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
        row![
            label_value("scx", &format!("{:02X}", scx)),
            label_value("scy", &format!("{:02X}", scy)),
        ]
        .spacing(s()),
        palette_row("bgp", bgp, pal),
    ]
    .spacing(xs())
    .into()
}

fn window_section(control: Control, ppu: &Ppu) -> Element<'static, Message> {
    let wx = ppu.read_register(missingno_gb::ppu::Register::WindowX);
    let wy = ppu.read_register(missingno_gb::ppu::Register::WindowY);

    column![
        row![
            enable_pip("win", control.window_enabled()),
            label_value("map", tile_map_addr(control.window_tile_map().0)),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
        row![
            label_value("wx", &format!("{:02X}", wx)),
            label_value("wy", &format!("{:02X}", wy)),
        ]
        .spacing(s()),
    ]
    .spacing(xs())
    .into()
}

fn sprites_section(
    control: Control,
    palettes: &missingno_gb::ppu::types::palette::Palettes,
    pal: &Palette,
) -> Element<'static, Message> {
    use missingno_gb::ppu::types::sprites::SpriteSize;

    column![
        row![
            enable_pip("sprites", control.sprites_enabled()),
            label_value("size", match control.sprite_size() {
                SpriteSize::Single => "8×8",
                SpriteSize::Double => "8×16",
            }),
        ]
        .spacing(s())
        .align_y(Vertical::Center),
        palette_row("obp0", palettes.sprite0.output(), pal),
        palette_row("obp1", palettes.sprite1.output(), pal),
    ]
    .spacing(xs())
    .into()
}

// --- Shared helpers ---

fn enable_pip(label: &str, enabled: bool) -> Element<'static, Message> {
    row![
        pip(enabled, palette::GREEN),
        text(label.to_owned())
            .font(fonts::monospace())
            .size(LABEL)
            .color(if enabled { palette::TEXT } else { palette::SURFACE2 }),
    ]
    .spacing(xs())
    .align_y(Vertical::Center)
    .into()
}

fn tile_map_addr(id: u8) -> &'static str {
    match id {
        0 => "9800",
        _ => "9C00",
    }
}

fn tile_addr(mode: TileAddressMode) -> &'static str {
    match mode {
        TileAddressMode::Block0Block1 => "8000",
        TileAddressMode::Block2Block1 => "8800",
    }
}

fn palette_row(label: &str, register_value: u8, pal: &Palette) -> Element<'static, Message> {
    let map = PaletteMap(register_value);

    row![
        container(
            text(label.to_owned())
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::MUTED)
        )
        .width(Length::Fixed(40.0)),
        palette_swatches(&map, pal),
        text(format!("{:02X}", register_value))
            .font(fonts::monospace())
            .size(LABEL)
            .color(palette::OVERLAY0),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn palette_swatches(map: &PaletteMap, pal: &Palette) -> Element<'static, Message> {
    use missingno_gb::ppu::types::palette::PaletteIndex;

    row![
        color_swatch(iced_color(map.color(PaletteIndex(0), pal))),
        color_swatch(iced_color(map.color(PaletteIndex(1), pal))),
        color_swatch(iced_color(map.color(PaletteIndex(2), pal))),
        color_swatch(iced_color(map.color(PaletteIndex(3), pal))),
    ]
    .spacing(2.0)
    .into()
}

fn color_swatch(color: Color) -> Element<'static, Message> {
    container(Space::new())
        .width(14.0)
        .height(14.0)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border::default()
                .rounded(2.0)
                .width(1.0)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)),
            ..Default::default()
        })
        .into()
}

// --- Shared label/value display ---

fn label_value(label: &str, value: &str) -> Element<'static, Message> {
    row![
        text(label.to_owned())
            .font(fonts::monospace())
            .size(LABEL)
            .color(palette::MUTED),
        text(value.to_owned())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::TEXT),
    ]
    .spacing(xs())
    .align_y(Vertical::Center)
    .into()
}
