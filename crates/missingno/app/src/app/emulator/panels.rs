//! Play-mode side panels and the icon rail that toggles them. Open panels
//! stack as titled, content-sized cards (the debugger sidebar's section look,
//! not its resizable pane grid); the rail is the single show/hide control.
//! Sections: Console (the machine's own controls, e.g. VCS switches),
//! Controllers (what each port carries and which host device plays it),
//! Display (the shared output surface, e.g. the DMG palette), and the Play log
//! (this session's screenshots and prints).

use std::fmt;

use iced::{
    Alignment::Center,
    Element,
    Length::{Fill, Fixed},
    widget::{button, column, container, image, pick_list, row, scrollable, text, toggler},
};
use missingno_core::ports::{PeripheralId, PortId};
use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};

use super::Message;
use crate::app::{
    self, automation, controls,
    emulation::SwitchLevels,
    library::activity,
    settings::view::{DisplayOptions, DisplayRow, Message as SettingsMessage},
    system::PanelControl,
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
    Controllers,
    Display,
    PlayLog,
}

impl PlayPanel {
    fn icon(self) -> Icon {
        match self {
            Self::Console => Icon::Sliders,
            Self::Controllers => Icon::Gamepad,
            Self::Display => Icon::Monitor,
            Self::PlayLog => Icon::Image,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Controllers => "Controllers",
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

/// A controller type a port takes, as a pick-list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerChoice {
    pub peripheral: PeripheralId,
    pub label: &'static str,
}

impl fmt::Display for ControllerChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

/// A console port on the play screen: the controller types it takes, and the one
/// in it now.
#[derive(Debug, Clone)]
pub struct PortSeat {
    pub port: PortId,
    pub label: &'static str,
    pub choices: Vec<ControllerChoice>,
    pub plugged: Option<PeripheralId>,
}

/// A host input device and the port it plays.
#[derive(Debug, Clone)]
pub struct DeviceSeat {
    pub source: controls::InputSource,
    /// The device as its driver names it.
    pub name: String,
    pub port: PortId,
}

impl DeviceSeat {
    /// This device's name in an automation id. A pad's gilrs id keeps identical
    /// twins apart; it lasts as long as the pads stay connected.
    fn id_name(&self) -> String {
        match self.source {
            controls::InputSource::Keyboard => "keyboard".to_string(),
            controls::InputSource::Gamepad(id) => format!("gamepad{id}"),
        }
    }
}

/// The machine's controller ports and the host devices playing them.
#[derive(Debug, Clone, Default)]
pub struct Controllers {
    pub ports: Vec<PortSeat>,
    pub devices: Vec<DeviceSeat>,
}

/// A port as an entry of a device's port pick list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PortChoice {
    port: PortId,
    label: &'static str,
}

impl fmt::Display for PortChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

/// Everything the panels render from, plus which sections this console offers.
pub struct PanelContext<'a> {
    pub switches: &'a [PanelControl],
    pub switch_levels: &'a SwitchLevels,
    pub controllers: &'a Controllers,
    pub display: DisplayOptions,
    pub play_log: &'a [PlayLogEntry<'a>],
    pub has_console: bool,
    pub has_controllers: bool,
    pub has_display: bool,
    pub has_playlog: bool,
}

/// The sections in the order they stack, each with whether this console offers
/// it — the machine itself, then what is plugged into it, then the output.
fn sections(ctx: &PanelContext) -> [(PlayPanel, bool); 4] {
    [
        (PlayPanel::Console, ctx.has_console),
        (PlayPanel::Controllers, ctx.has_controllers),
        (PlayPanel::Display, ctx.has_display),
        (PlayPanel::PlayLog, ctx.has_playlog),
    ]
}

/// The vertical icon rail. Content-driven: an icon appears only for a section
/// that has something to show. Returns `None` when none do, so the caller
/// omits the rail entirely.
pub fn rail(open: &[PlayPanel], ctx: &PanelContext) -> Option<Element<'static, app::Message>> {
    let available = sections(ctx);
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
    let mut col = column![].spacing(s());
    let mut any = false;
    for (panel, available) in sections(ctx) {
        if available && open.contains(&panel) {
            any = true;
            let body = match panel {
                PlayPanel::Console => console_body(ctx.switches, ctx.switch_levels),
                PlayPanel::Controllers => controllers_body(ctx.controllers),
                PlayPanel::Display => display_body(&ctx.display),
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

fn console_body(
    switches: &[PanelControl],
    levels: &SwitchLevels,
) -> Element<'static, app::Message> {
    let mut col = column![].spacing(m());
    for (index, switch) in switches.iter().enumerate() {
        let Some((positions, default_high)) = switch.toggle() else {
            continue;
        };
        let level = levels.level(switch.role).unwrap_or(default_high);
        let position = positions[level as usize];
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

/// What each port carries, then which port each host device plays: the two
/// choices that make a second player.
fn controllers_body(controllers: &Controllers) -> Element<'static, app::Message> {
    let ports: Vec<PortChoice> = controllers
        .ports
        .iter()
        .map(|seat| PortChoice {
            port: seat.port,
            label: seat.label,
        })
        .collect();

    let mut col = column![].spacing(m());
    for seat in &controllers.ports {
        let port = seat.port;
        let selected = seat.plugged.and_then(|plugged| {
            seat.choices
                .iter()
                .find(|choice| choice.peripheral == plugged)
                .cloned()
        });
        col = col.push(
            column![
                text(seat.label),
                automation::tag(
                    &automation::ids::controllers_port(port),
                    pick_list(seat.choices.clone(), selected, move |choice| {
                        app::Message::PlugPeripheral(port, choice.peripheral)
                    })
                    .width(Fill),
                ),
            ]
            .spacing(xs()),
        );
    }

    col = col.push(app_text::label("Devices"));
    for seat in &controllers.devices {
        let source = seat.source;
        let selected = ports
            .iter()
            .find(|choice| choice.port == seat.port)
            .cloned();
        col = col.push(
            column![
                text(seat.name.clone()),
                automation::tag(
                    &automation::ids::controllers_device(&seat.id_name()),
                    pick_list(ports.clone(), selected, move |choice| {
                        app::Message::AssignDevice(source, choice.port)
                    })
                    .width(Fill),
                ),
            ]
            .spacing(xs()),
        );
    }
    col.into()
}

/// One pick list of the Controllers section: its id and its accessible name.
/// The section is pick lists throughout, so nothing here has a press action.
pub(in crate::app) struct ControllersElement {
    pub id: String,
    pub label: String,
}

/// Every pick list the Controllers section shows, in reading order: each port's
/// controller type, then the port each host device plays.
pub(in crate::app) fn controllers_elements(controllers: &Controllers) -> Vec<ControllersElement> {
    controllers
        .ports
        .iter()
        .map(|seat| ControllersElement {
            id: automation::ids::controllers_port(seat.port),
            label: format!("Choose the {}", seat.label),
        })
        .chain(controllers.devices.iter().map(|seat| ControllersElement {
            id: automation::ids::controllers_device(&seat.id_name()),
            label: format!("Choose the port {} plays", seat.name),
        }))
        .collect()
}

/// Quick access to the display settings, filtered to the running console: the
/// effects its screen shows, then the colour options its games carry. The rows
/// set the same settings the Display section does — they are not overrides.
fn display_body(options: &DisplayOptions) -> Element<'static, app::Message> {
    let mut col = column![].spacing(m());
    for entry in options.effect_rows() {
        col = col.push(display_switch(entry));
    }

    let game_boy = options.game_boy_rows();
    if !game_boy.is_empty() {
        col = col.push(app_text::label("Game Boy"));
        for entry in game_boy {
            col = col.push(match entry {
                DisplayRow::Palette { choice, selected } => {
                    palette_tile(&entry.id_name(), choice, selected, entry.activate())
                }
                _ => display_switch(entry),
            });
        }
    }
    col.into()
}

fn display_switch(entry: DisplayRow) -> Element<'static, app::Message> {
    automation::tag(
        &automation::ids::display_row(&entry.id_name()),
        toggler(entry.switched_on().unwrap_or(false))
            .label(entry.label())
            .on_toggle(move |enabled| entry.set(enabled).into())
            .size(m()),
    )
}

/// One palette as a full-width row: its four shades beside its name.
fn palette_tile(
    id_name: &str,
    choice: PaletteChoice,
    selected: bool,
    select: SettingsMessage,
) -> Element<'static, app::Message> {
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
    let tile = if selected {
        buttons::selected_raw(tile_content)
    } else {
        buttons::subtle_raw(tile_content).on_press(select.into())
    };
    automation::tag(&automation::ids::display_row(id_name), tile.width(Fill))
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
                    buttons::subtle(icons::m(Icon::Save))
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
