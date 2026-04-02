use iced::{
    Color, Element,
    Length::Fill,
    widget::{column, container, image, scrollable, text},
};

use crate::app::{
    self,
    core::{
        sizes::{l, m},
        text as app_text,
    },
    library::GameEntry,
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

const PANEL_WIDTH: f32 = 320.0;

pub fn view<'a>(
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

    col = col.push(app_text::xl(&entry.title));

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

    if let Some(description) = &entry.description {
        col = col.push(text(description.as_str()));
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
