//! Play-mode side panels and the icon rail that toggles them. Open panels
//! stack as titled, content-sized cards (the debugger sidebar's section look,
//! not its resizable pane grid); the rail is the single show/hide control.
//! Sections: Console (the machine's own controls, e.g. VCS switches), Display
//! (the shared output surface, e.g. the DMG palette), and the Play log
//! (this session's screenshots and prints).

use iced::{
    Alignment::Center,
    Element,
    Length::{Fill, Fixed},
    widget::{button, column, container, image, row, scrollable, text, toggler},
};
use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};

use super::Message;
use crate::app::{
    self,
    library::activity,
    system::ConsoleSwitch,
    ui::{
        buttons, fonts,
        icons::{self, Icon},
        palette::{self, MUTED},
        sizes::{m, s, xs},
        text as app_text,
    },
};

const PANEL_WIDTH: f32 = 260.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayPanel {
    Console,
    Display,
    PlayLog,
}

impl PlayPanel {
    fn icon(self) -> Icon {
        match self {
            Self::Console => Icon::Sliders,
            Self::Display => Icon::Monitor,
            Self::PlayLog => Icon::Image,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Display => "Display",
            Self::PlayLog => "Play log",
        }
    }
}

/// A screenshot or print captured during the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Screenshot,
    Print,
}

impl CaptureKind {
    fn label(self) -> &'static str {
        match self {
            Self::Screenshot => "Screenshot",
            Self::Print => "Print",
        }
    }
}

/// One entry in the live play log: a cached thumbnail plus when and what it is.
pub struct PlayLogEntry<'a> {
    pub kind: CaptureKind,
    pub handle: &'a image::Handle,
    pub at: jiff::Timestamp,
    /// Index into the session's events, for export.
    pub event_index: usize,
}

/// Everything the panels render from, plus which sections this console offers.
pub struct PanelContext<'a> {
    pub switches: &'a [ConsoleSwitch],
    pub switch_levels: &'a [bool],
    pub palette: PaletteChoice,
    pub use_sgb_colors: bool,
    pub play_log: &'a [PlayLogEntry<'a>],
    pub has_console: bool,
    pub has_display: bool,
    pub has_playlog: bool,
}

/// The vertical icon rail. Content-driven: an icon appears only for a section
/// that has something to show. Returns `None` when none do, so the caller
/// omits the rail entirely.
pub fn rail(open: &[PlayPanel], ctx: &PanelContext) -> Option<Element<'static, app::Message>> {
    let available = [
        (PlayPanel::Console, ctx.has_console),
        (PlayPanel::Display, ctx.has_display),
        (PlayPanel::PlayLog, ctx.has_playlog),
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
                open.contains(&panel),
                Message::TogglePanel(panel).into(),
            ));
        }
    }
    Some(container(col).padding([s(), xs()]).into())
}

/// The open panels, stacked as titled cards that size to their content, in a
/// fixed-width scrollable column docked beside the screen. Renders in a stable
/// order regardless of the order they were toggled on.
pub fn side_column(
    open: &[PlayPanel],
    ctx: &PanelContext,
) -> Option<Element<'static, app::Message>> {
    let order = [
        (PlayPanel::Console, ctx.has_console),
        (PlayPanel::Display, ctx.has_display),
        (PlayPanel::PlayLog, ctx.has_playlog),
    ];

    let mut col = column![].spacing(s());
    let mut any = false;
    for (panel, available) in order {
        if available && open.contains(&panel) {
            any = true;
            let body = match panel {
                PlayPanel::Console => console_body(ctx.switches, ctx.switch_levels),
                PlayPanel::Display => display_body(ctx.palette, ctx.use_sgb_colors),
                PlayPanel::PlayLog => playlog_body(ctx.play_log),
            };
            col = col.push(section_card(panel.title(), body));
        }
    }
    if !any {
        return None;
    }
    Some(
        container(scrollable(col.padding(s())).height(Fill))
            .width(Fixed(PANEL_WIDTH))
            .height(Fill)
            .into(),
    )
}

/// A titled, content-sized card, mirroring the debugger sidebar's section look.
fn section_card(
    title: &'static str,
    body: Element<'static, app::Message>,
) -> Element<'static, app::Message> {
    let header = container(text(title).font(fonts::title()).size(13.0).color(MUTED))
        .width(Fill)
        .padding([xs(), s()])
        .style(section_header_style);

    container(column![header, container(body).padding(s())].width(Fill))
        .width(Fill)
        .style(section_style)
        .into()
}

fn section_style(theme: &iced::Theme) -> container::Style {
    let pal = theme.extended_palette();
    container::Style {
        background: Some(pal.background.base.color.into()),
        border: iced::Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
        ..Default::default()
    }
}

fn section_header_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.03).into()),
        ..Default::default()
    }
}

fn console_body(switches: &[ConsoleSwitch], levels: &[bool]) -> Element<'static, app::Message> {
    let mut col = column![].spacing(m());
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

fn playlog_body(entries: &[PlayLogEntry]) -> Element<'static, app::Message> {
    let mut col = column![
        buttons::standard(
            row![icons::m(Icon::Camera), text("Screenshot")]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(app::Message::TakeScreenshot),
    ]
    .spacing(m());

    if entries.is_empty() {
        col = col.push(app_text::detail("No captures yet").color(MUTED));
        return col.into();
    }

    // Newest first, so the most recent capture is at the top of the log.
    for entry in entries.iter().rev() {
        let caption = format!(
            "{} · {}",
            entry.kind.label(),
            activity::format_local_time(&entry.at)
        );
        col = col.push(
            column![
                image(entry.handle.clone()).width(Fill),
                row![
                    app_text::detail(caption).color(MUTED).width(Fill),
                    buttons::subtle(icons::m(Icon::Download))
                        .on_press(app::Message::ExportCapture(entry.event_index)),
                ]
                .align_y(Center),
            ]
            .spacing(xs()),
        );
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
