use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self,
    core::{
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
};

use crate::app::library::GameEntry;

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

const PANEL_WIDTH: f32 = 320.0;

#[allow(private_interfaces)]
pub(crate) fn view<'a>(
    entry: &'a GameEntry,
    cover: Option<&'a image::Handle>,
) -> Element<'a, app::Message> {
    let mut col = column![].spacing(m());

    if let Some(handle) = cover {
        col = col.push(
            image(handle.clone())
                .width(PANEL_WIDTH - l() * 2.0)
                .content_fit(iced::ContentFit::ScaleDown),
        );
    }

    col = col.push(app_text::xl(entry.display_title()));

    // Subtitle: publisher · year · platform
    let subtitle_parts: Vec<&str> = [
        entry.publisher.as_deref(),
        entry.year.as_deref(),
        entry.platform.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        col = col.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Links
    if entry.wikipedia_url.is_some() || entry.igdb_url.is_some() {
        let mut links = row![].spacing(m());

        if let Some(url) = &entry.wikipedia_url {
            links = links.push(
                mouse_area(
                    row![icons::m(Icon::Globe), text("Wikipedia").color(MUTED)]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::OpenUrl(leak_str(url)))
                .interaction(mouse::Interaction::Pointer),
            );
        }

        if let Some(url) = &entry.igdb_url {
            links = links.push(
                mouse_area(
                    row![icons::m(Icon::Globe), text("IGDB").color(MUTED)]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::OpenUrl(leak_str(url)))
                .interaction(mouse::Interaction::Pointer),
            );
        }

        col = col.push(links);
    }

    container(scrollable(col.padding(l()).max_width(PANEL_WIDTH)))
        .width(PANEL_WIDTH)
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

/// Leak a string to get a `&'static str` for use in messages.
/// This is acceptable because there are a bounded number of game entries.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
