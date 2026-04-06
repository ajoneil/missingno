use std::path::PathBuf;

use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{column, container, mouse_area, row, svg, text, toggler},
};

use crate::app::{
    self, controls,
    core::{
        buttons, horizontal_rule,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    settings::{Action, Bindings, EMULATOR_ACTIONS, GB_ACTIONS},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    General,
    Display,
    Controls,
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

// Catppuccin Mocha "subtext0" — readable but clearly secondary
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

pub fn view(
    settings: &super::settings::Settings,
    section: Section,
    listening_for: Option<ListeningFor>,
) -> Element<'_, app::Message> {
    let sidebar = sidebar_view(section);
    let content = match section {
        Section::Display => display_section(settings),
        Section::General => general_section(settings),
        Section::Controls => controls_section(settings, listening_for),
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
    ];

    let mut col = column![].spacing(s());

    for (section, icon, label) in sections {
        let btn = if section == current {
            let label_row = row![
                icons::m(icon).style(|_, _| iced::widget::svg::Style {
                    color: Some(iced::Color::WHITE)
                }),
                text(label).color(iced::Color::WHITE),
            ]
            .spacing(s())
            .align_y(Center);
            buttons::primary(label_row).width(Fill)
        } else {
            let label_row = row![icons::m(icon), text(label)]
                .spacing(s())
                .align_y(Center);
            buttons::subtle(label_row)
                .on_press(Message::SelectSection(section).into())
                .width(Fill)
        };

        col = col.push(btn);
    }

    container(col.padding(m()))
        .width(220)
        .height(Fill)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                ..Default::default()
            }
        })
        .into()
}

fn controls_section(
    settings: &super::settings::Settings,
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

    row![label, binding_btn]
        .spacing(s())
        .align_y(Center)
        .into()
}

fn display_section(settings: &super::settings::Settings) -> Element<'_, app::Message> {
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

        let label = if is_selected {
            text(format!("{choice}")).color(iced::Color::WHITE)
        } else {
            text(format!("{choice}"))
        };

        let tile_content = column![swatches, label,].spacing(s()).align_x(Center);

        let tile = if is_selected {
            buttons::primary_raw(tile_content)
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

fn general_section(settings: &super::settings::Settings) -> Element<'_, app::Message> {
    let version = env!("CARGO_PKG_VERSION").trim_end_matches(".0");

    let about = row![
        icons::xl(Icon::GameBoy)
            .width(64)
            .height(64)
            .style(|_, _| svg::Style { color: None }),
        column![
            app_text::heading("Missingno"),
            row![
                text(format!("Version {version}")).color(MUTED),
                text("·").color(MUTED),
                mouse_area(text("Website").color(MUTED))
                    .on_press(app::Message::OpenUrl("https://andyofniall.net/"))
                    .interaction(mouse::Interaction::Pointer),
                text("·").color(MUTED),
                mouse_area(text("GitHub").color(MUTED))
                    .on_press(app::Message::OpenUrl(
                        "https://github.com/ajoneil/missingno"
                    ))
                    .interaction(mouse::Interaction::Pointer),
            ]
            .spacing(s()),
        ]
        .spacing(s()),
    ]
    .spacing(m())
    .align_y(Center);

    let network = column![
        toggler(settings.internet_enabled)
            .label("Allow internet access")
            .on_toggle(|enabled| Message::SetInternetEnabled(enabled).into())
            .size(m()),
        row![
            text("Game metadata provided by").color(MUTED),
            mouse_area(text("Hasheous").color(MUTED))
                .on_press(app::Message::OpenUrl("https://hasheous.org"))
                .interaction(mouse::Interaction::Pointer),
        ]
        .spacing(s()),
    ]
    .spacing(s());

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
