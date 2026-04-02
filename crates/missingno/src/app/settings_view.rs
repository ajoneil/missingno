use std::path::PathBuf;

use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{column, container, mouse_area, row, svg, text, toggler},
};

use crate::app::{
    self,
    core::{
        buttons, horizontal_rule,
        icons::{self, Icon},
        sizes::{l, m, s, xl},
        text as app_text,
    },
};

#[derive(Debug, Clone)]
pub enum Message {
    SetInternetEnabled(bool),
    PickRomDirectory,
    AddRomDirectory(PathBuf),
    RemoveRomDirectory(usize),
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

pub fn view(settings: &super::settings::Settings) -> Element<'_, app::Message> {
    let version = env!("CARGO_PKG_VERSION").trim_end_matches(".0");

    let header = row![
        icons::xl(Icon::GameBoy)
            .width(64)
            .height(64)
            .style(|_, _| svg::Style { color: None }),
        column![
            app_text::xl("Missingno"),
            row![
                text(format!("Version {version}")).color(MUTED),
                text("·").color(MUTED),
                mouse_area(text("Website").color(MUTED))
                    .on_press(app::Message::OpenUrl("https://andyofniall.net/"))
                    .interaction(mouse::Interaction::Pointer),
                text("·").color(MUTED),
                mouse_area(text("GitHub").color(MUTED))
                    .on_press(app::Message::OpenUrl("https://github.com/ajoneil/missingno"))
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
        text("When enabled, Missingno will look up game metadata and cover art using ROM checksums.")
            .color(MUTED),
    ]
    .spacing(s());

    let mut directories = column![app_text::m("ROM Folders")].spacing(s());

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

    directories = directories.push(
        buttons::standard("Add folder...")
            .on_press(Message::PickRomDirectory.into()),
    );

    column![
        container(
            buttons::subtle(
                row![icons::m(Icon::Back), text("Back")]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(Message::Back.into()),
        )
        .padding(m()),
        horizontal_rule(),
        container(
            column![header, horizontal_rule(), network, horizontal_rule(), directories]
                .spacing(l())
                .max_width(500),
        )
        .center_x(Fill)
        .padding([xl(), m()]),
    ]
    .height(Fill)
    .into()
}
