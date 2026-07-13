//! Play-mode side panels and the icon rail that toggles them. Borrows the
//! debugger's rail affordance (tooltip'd icon toggles, highlighted when
//! active) without its pane machinery. Two sections: Console (the machine's
//! own controls, e.g. VCS switches) and Display (the shared output surface,
//! e.g. the DMG palette).

use iced::{
    Alignment::Center,
    Element,
    Length::{Fill, Fixed},
    widget::{button, column, container, row, scrollable, text, toggler},
};
use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};

use super::Message;
use crate::app::{
    self,
    system::ConsoleSwitch,
    ui::{
        buttons, containers, fonts,
        icons::{self, Icon},
        palette,
        sizes::{m, s, xs},
        text as app_text,
    },
};

const PANEL_WIDTH: f32 = 260.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayPanel {
    Console,
    Display,
}

impl PlayPanel {
    fn icon(self) -> Icon {
        match self {
            Self::Console => Icon::Sliders,
            Self::Display => Icon::Monitor,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Display => "Display",
        }
    }
}

/// The vertical icon rail. Content-driven: an icon appears only for a section
/// that has something to show. Returns `None` when neither does, so the caller
/// omits the rail entirely.
pub fn rail(
    open: Option<PlayPanel>,
    has_console: bool,
    has_display: bool,
) -> Option<Element<'static, app::Message>> {
    let available = [
        (PlayPanel::Console, has_console),
        (PlayPanel::Display, has_display),
    ];
    if available.iter().all(|(_, show)| !show) {
        return None;
    }

    let mut col = column![].spacing(xs());
    for (panel, show) in available {
        if show {
            col = col.push(rail_icon(
                panel.icon(),
                panel.title(),
                open == Some(panel),
                Message::TogglePanel(panel).into(),
            ));
        }
    }
    Some(container(col).padding([s(), xs()]).into())
}

/// The open panel's body, docked beside the screen.
pub fn body(
    panel: PlayPanel,
    switches: &[ConsoleSwitch],
    switch_levels: &[bool],
    current_palette: PaletteChoice,
    use_sgb_colors: bool,
) -> Element<'static, app::Message> {
    let content = match panel {
        PlayPanel::Console => console_body(switches, switch_levels),
        PlayPanel::Display => display_body(current_palette, use_sgb_colors),
    };
    container(scrollable(container(content).padding(m())).height(Fill))
        .width(Fixed(PANEL_WIDTH))
        .height(Fill)
        .style(containers::sidebar)
        .into()
}

fn console_body(switches: &[ConsoleSwitch], levels: &[bool]) -> Element<'static, app::Message> {
    let mut col = column![app_text::label("Console")].spacing(m());
    for (index, switch) in switches.iter().enumerate() {
        let level = levels.get(index).copied().unwrap_or(switch.default_high);
        let position = switch.positions[level as usize];
        col = col.push(
            row![
                text(switch.label).width(Fill),
                button(text(position))
                    .on_press(Message::ToggleSwitch(index).into())
                    .style(button::secondary),
            ]
            .spacing(s())
            .align_y(Center),
        );
    }
    col.into()
}

fn display_body(current: PaletteChoice, use_sgb_colors: bool) -> Element<'static, app::Message> {
    use crate::app::settings::view::Message as SettingsMessage;

    let mut col = column![
        app_text::label("Display"),
        toggler(use_sgb_colors)
            .label("Super Game Boy colours")
            .on_toggle(|enabled| SettingsMessage::SetUseSgbColors(enabled).into())
            .size(m()),
        app_text::label("Palette"),
    ]
    .spacing(m());

    for &choice in PaletteChoice::ALL {
        let pal = choice.palette();
        let swatches = row![
            swatch(pal.color(PaletteIndex(0))),
            swatch(pal.color(PaletteIndex(1))),
            swatch(pal.color(PaletteIndex(2))),
            swatch(pal.color(PaletteIndex(3))),
        ];
        let tile_content = row![swatches, text(format!("{choice}"))]
            .spacing(s())
            .align_y(Center);
        let tile = if current == choice {
            buttons::selected_raw(tile_content)
        } else {
            buttons::subtle_raw(tile_content)
                .on_press(SettingsMessage::SelectPalette(choice).into())
        };
        col = col.push(tile.width(Fill));
    }
    col.into()
}

fn swatch(color: rgb::RGB8) -> Element<'static, app::Message> {
    let c = iced::Color::from_rgb8(color.r, color.g, color.b);
    container(iced::widget::Space::new().width(20).height(20))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(c.into()),
            ..Default::default()
        })
        .into()
}

fn rail_icon(
    icon: Icon,
    label: &'static str,
    active: bool,
    message: app::Message,
) -> Element<'static, app::Message> {
    use iced::widget::tooltip;

    let color = if active {
        palette::PURPLE
    } else {
        palette::SURFACE2
    };
    let btn: Element<'_, app::Message> = button(icons::m_colored(icon, color))
        .on_press(message)
        .style(button::text)
        .into();

    tooltip(
        btn,
        container(text(label).font(fonts::monospace()).size(13.0))
            .padding([2.0, s()])
            .style(tooltip_style),
        tooltip::Position::Left,
    )
    .into()
}

fn tooltip_style(theme: &iced::Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: iced::Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(palette.background.strong.color),
        ..Default::default()
    }
}
