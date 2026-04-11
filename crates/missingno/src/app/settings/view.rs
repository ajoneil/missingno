use std::path::PathBuf;

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    widget::{column, container, row, svg, text, toggler},
};

use crate::app::{
    self, controls,
    settings::{Action, Bindings, EMULATOR_ACTIONS, GB_ACTIONS},
    ui::{
        buttons, containers, horizontal_rule,
        icons::{self, Icon},
        palette::MUTED,
        sizes::{l, m, s},
        text as app_text,
    },
};

use app_text::TextPart;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    General,
    Display,
    Controls,
    Hardware,
}

/// What binding slot we're waiting for input on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListeningFor {
    Keyboard(Action),
    Gamepad(Action),
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectSection(Section),
    SetInternetEnabled(bool),
    PickRomDirectory,
    AddRomDirectory(PathBuf),
    RemoveRomDirectory(usize),
    SelectPalette(missingno_gb::ppu::types::palette::PaletteChoice),
    SetUseSgbColors(bool),
    SetHasheousEnabled(bool),
    SetHomebrewHubEnabled(bool),
    SetCartridgeRwEnabled(bool),
    StartListening(ListeningFor),
    CaptureBinding(String),
    ClearBinding,
    CancelCapture,
    ResetBindings,
    Back,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Settings(message)
    }
}

pub(in crate::app) fn view<'a>(
    settings: &'a super::Settings,
    section: Section,
    listening_for: Option<ListeningFor>,
    detected_cartridge_devices: &'a [crate::cartridge_rw::DetectedDevice],
) -> Element<'a, app::Message> {
    let sidebar = sidebar_view(section);
    let content = match section {
        Section::Display => display_section(settings),
        Section::General => general_section(settings),
        Section::Controls => controls_section(settings, listening_for),
        Section::Hardware => hardware_section(settings, detected_cartridge_devices),
    };

    column![
        row![
            buttons::subtle(icons::m(Icon::Back)).on_press(Message::Back.into()),
            app_text::heading("Settings"),
        ]
        .spacing(s())
        .padding(m())
        .align_y(Center),
        horizontal_rule(),
        row![sidebar, content].height(Fill),
    ]
    .height(Fill)
    .into()
}

fn sidebar_view(current: Section) -> Element<'static, app::Message> {
    let sections = [
        (Section::General, Icon::Sliders, "General"),
        (Section::Display, Icon::Monitor, "Display"),
        (Section::Controls, Icon::Gamepad, "Controls"),
        (Section::Hardware, Icon::CircuitBoard, "Hardware"),
    ];

    let mut col = column![].spacing(s());

    for (section, icon, label) in sections {
        let label_row = row![icons::m(icon), text(label)]
            .spacing(s())
            .align_y(Center);
        let btn = if section == current {
            buttons::selected(label_row).width(Fill)
        } else {
            buttons::subtle(label_row)
                .on_press(Message::SelectSection(section).into())
                .width(Fill)
        };

        col = col.push(btn);
    }

    container(col.padding(m()))
        .width(220)
        .height(Fill)
        .style(containers::sidebar)
        .into()
}

fn controls_section(
    settings: &super::Settings,
    listening_for: Option<ListeningFor>,
) -> Element<'_, app::Message> {
    let keyboard_col = binding_column(
        "Keyboard",
        &settings.keyboard_bindings,
        listening_for,
        |action| ListeningFor::Keyboard(action),
        |s| controls::display_key_name(s).to_string(),
    );

    let gamepad_col = binding_column(
        "Controller",
        &settings.gamepad_bindings,
        listening_for,
        |action| ListeningFor::Gamepad(action),
        |s| controls::display_gamepad_name(s).to_string(),
    );

    let content = column![
        row![keyboard_col, gamepad_col].spacing(l()),
        horizontal_rule(),
        buttons::standard("Reset to defaults").on_press(Message::ResetBindings.into()),
    ]
    .spacing(m())
    .max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

fn binding_column<'a>(
    title: &'a str,
    bindings: &Bindings,
    listening_for: Option<ListeningFor>,
    make_target: impl Fn(Action) -> ListeningFor,
    display_name: impl Fn(&str) -> String,
) -> Element<'a, app::Message> {
    let mut col = column![app_text::label(title)].spacing(s());

    // Game Boy buttons
    col = col.push(section_header("Game Boy"));
    for &action in &GB_ACTIONS {
        col = col.push(binding_row(
            action,
            bindings,
            listening_for,
            &make_target,
            &display_name,
        ));
    }

    // Emulator actions
    col = col.push(section_header("Emulator"));
    for &action in &EMULATOR_ACTIONS {
        col = col.push(binding_row(
            action,
            bindings,
            listening_for,
            &make_target,
            &display_name,
        ));
    }

    col.into()
}

fn section_header(label: &'static str) -> Element<'static, app::Message> {
    column![
        iced::widget::Space::new().height(s()),
        app_text::label(label),
    ]
    .into()
}

fn binding_row(
    action: Action,
    bindings: &Bindings,
    listening_for: Option<ListeningFor>,
    make_target: &impl Fn(Action) -> ListeningFor,
    display_name: &impl Fn(&str) -> String,
) -> Element<'static, app::Message> {
    let target = make_target(action);
    let is_listening = listening_for == Some(target);

    let label = text(format!("{action}")).width(120);

    let binding_btn = if is_listening {
        buttons::primary(text("Press key…").color(iced::Color::WHITE)).width(150)
    } else {
        let display = bindings
            .get(action)
            .map(|s| display_name(s))
            .unwrap_or_else(|| "—".to_string());
        buttons::standard(text(display))
            .on_press(Message::StartListening(target).into())
            .width(150)
    };

    row![label, binding_btn].spacing(s()).align_y(Center).into()
}

fn display_section(settings: &super::Settings) -> Element<'_, app::Message> {
    use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};

    let mut content = column![
        toggler(settings.use_sgb_colors)
            .label("Use Super Game Boy colours for supported games")
            .on_toggle(|enabled| Message::SetUseSgbColors(enabled).into())
            .size(m()),
        text("When disabled, the default palette is used for all games.").color(MUTED),
        horizontal_rule(),
        app_text::label("Palette"),
    ]
    .spacing(m());

    // Palette selector as color tiles
    let mut palette_row = row![].spacing(m());

    for &choice in PaletteChoice::ALL {
        let palette = choice.palette();
        let is_selected = settings.palette == choice;

        let swatches = row![
            color_swatch(palette.color(PaletteIndex(0))),
            color_swatch(palette.color(PaletteIndex(1))),
            color_swatch(palette.color(PaletteIndex(2))),
            color_swatch(palette.color(PaletteIndex(3))),
        ]
        .spacing(0);

        let label = text(format!("{choice}"));
        let tile_content = column![swatches, label,].spacing(s()).align_x(Center);

        let tile = if is_selected {
            buttons::selected_raw(tile_content)
        } else {
            buttons::subtle_raw(tile_content).on_press(Message::SelectPalette(choice).into())
        };

        palette_row = palette_row.push(tile);
    }

    content = content.push(palette_row);
    let content = content.max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

fn color_swatch(color: rgb::RGB8) -> Element<'static, app::Message> {
    let c = iced::Color::from_rgb8(color.r, color.g, color.b);
    container(iced::widget::Space::new().width(40).height(40))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(c.into()),
            ..Default::default()
        })
        .into()
}

fn general_section(settings: &super::Settings) -> Element<'_, app::Message> {
    let version = env!("CARGO_PKG_VERSION").trim_end_matches(".0");

    let about = row![
        icons::xl(Icon::GameBoy)
            .width(64)
            .height(64)
            .style(|_, _| svg::Style { color: None }),
        column![
            app_text::heading("Missingno"),
            app_text::link_text(
                [
                    TextPart::plain(format!("Version {version} · ")),
                    TextPart::link("Website", "https://andyofniall.net/"),
                    TextPart::plain(" · "),
                    TextPart::link("GitHub", "https://github.com/ajoneil/missingno"),
                ],
                MUTED
            ),
        ]
        .spacing(s()),
    ]
    .spacing(m())
    .align_y(Center);

    let mut network = column![
        toggler(settings.internet_enabled)
            .label("Allow internet access")
            .on_toggle(|enabled| Message::SetInternetEnabled(enabled).into())
            .size(m()),
    ]
    .spacing(m());

    if settings.internet_enabled {
        network = network.push(
            column![
                toggler(settings.hasheous_enabled)
                    .label("Game metadata")
                    .on_toggle(|enabled| Message::SetHasheousEnabled(enabled).into())
                    .size(m()),
                app_text::link_text(
                    [
                        TextPart::plain("Provided by "),
                        TextPart::link("Hasheous", "https://hasheous.org"),
                    ],
                    MUTED
                ),
            ]
            .spacing(s()),
        );

        network = network.push(
            column![
                toggler(settings.homebrew_hub_enabled)
                    .label("Homebrew catalogue")
                    .on_toggle(|enabled| Message::SetHomebrewHubEnabled(enabled).into())
                    .size(m()),
                app_text::link_text(
                    [
                        TextPart::plain("Browse free games from "),
                        TextPart::link("Homebrew Hub", "https://hh.gbdev.io"),
                    ],
                    MUTED
                ),
            ]
            .spacing(s()),
        );
    }

    let mut directories = column![].spacing(s());

    for (i, dir) in settings.rom_directories.iter().enumerate() {
        directories = directories.push(
            row![
                text(dir.to_string_lossy().to_string()).width(Fill),
                buttons::danger(icons::m(Icon::Close))
                    .on_press(Message::RemoveRomDirectory(i).into()),
            ]
            .spacing(s())
            .align_y(Center),
        );
    }

    directories = directories
        .push(buttons::standard("Add folder...").on_press(Message::PickRomDirectory.into()));

    let content = column![
        about,
        horizontal_rule(),
        app_text::label("Network"),
        network,
        horizontal_rule(),
        app_text::label("ROM Folders"),
        directories,
    ]
    .spacing(m())
    .max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

fn hardware_section<'a>(
    settings: &'a super::Settings,
    detected_devices: &'a [crate::cartridge_rw::DetectedDevice],
) -> Element<'a, app::Message> {
    let mut content = column![
        app_text::label("Cartridge Reader/Writer"),
        toggler(settings.cartridge_rw_enabled)
            .label("Enable cartridge reader/writer support")
            .on_toggle(|enabled| Message::SetCartridgeRwEnabled(enabled).into())
            .size(m()),
        app_text::link_text(
            [
                TextPart::plain(
                    "Read and write ROMs and save data from physical Game Boy cartridges using a "
                ),
                TextPart::link("GBxCart RW", "https://www.gbxcart.com/"),
                TextPart::plain(" device."),
            ],
            MUTED
        ),
        app_text::link_text(
            [
                TextPart::plain("For advanced features and broader hardware support, see "),
                TextPart::link("FlashGBX", "https://github.com/lesserkuma/FlashGBX"),
                TextPart::plain("."),
            ],
            MUTED
        ),
    ]
    .spacing(m());

    if settings.cartridge_rw_enabled {
        content = content.push(horizontal_rule());
        content = content.push(app_text::label("Detected Devices"));

        if detected_devices.is_empty() {
            content = content.push(
                text("No devices found. Devices will appear here automatically when connected.")
                    .color(MUTED),
            );
            if cfg!(target_os = "linux") {
                content = content.push(app_text::link_text(
                    [
                        TextPart::plain("You may need to install "),
                        TextPart::link(
                            "udev rules",
                            "https://github.com/ajoneil/missingno#cartridge-readerwriter",
                        ),
                        TextPart::plain(" for the device to be accessible."),
                    ],
                    MUTED,
                ));
            }
        } else {
            for device in detected_devices {
                content = content.push(
                    row![
                        icons::m(Icon::CircuitBoard),
                        column![
                            text(device.display_name()),
                            text(format!(
                                "{} (PCB v{}, FW v{})",
                                device.port_name, device.pcb_version, device.firmware_version
                            ))
                            .color(MUTED),
                        ]
                        .spacing(2),
                    ]
                    .spacing(s())
                    .align_y(Center),
                );
            }
        }
    }

    let content = content.max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}
