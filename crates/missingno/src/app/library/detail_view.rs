use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self, CurrentGame, Game,
    core::{
        buttons,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

#[allow(private_interfaces)]
pub(crate) fn view<'a>(
    current: &'a CurrentGame,
    game: &'a Game,
) -> Element<'a, app::Message> {
    let mut col = column![].spacing(l()).max_width(500);

    // Cover art
    if let Some(handle) = &current.cover {
        col = col.push(
            container(
                image(handle.clone())
                    .width(280)
                    .content_fit(iced::ContentFit::ScaleDown),
            )
            .center_x(Fill),
        );
    }

    // Play / Resume button
    let play_label = if matches!(game, Game::Loaded(_)) {
        "Resume"
    } else {
        "Play"
    };
    col = col.push(
        container(buttons::primary(play_label).on_press(app::Message::PlayFromDetail))
            .center_x(Fill),
    );

    // Title
    col = col.push(app_text::xl(current.entry.display_title()));

    // Subtitle: publisher · year · platform
    let subtitle_parts: Vec<&str> = [
        current.entry.publisher.as_deref(),
        current.entry.year.as_deref(),
        current.entry.platform.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        col = col.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Links
    if current.entry.wikipedia_url.is_some() || current.entry.igdb_url.is_some() {
        let mut links = row![].spacing(m());

        if let Some(url) = &current.entry.wikipedia_url {
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

        if let Some(url) = &current.entry.igdb_url {
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

    // Play stats
    let play_time = current.play_log.format_play_time();
    let sessions = current.play_log.sessions.len();
    let saves = current.play_log.save_events.len();

    let mut stats = column![].spacing(s());
    stats = stats.push(text(format!("Play time: {play_time}")).color(MUTED));
    if sessions > 0 {
        stats = stats.push(
            text(format!(
                "{sessions} session{}, {saves} save{}",
                if sessions == 1 { "" } else { "s" },
                if saves == 1 { "" } else { "s" },
            ))
            .color(MUTED),
        );
    }
    if let Some(first) = &current.play_log.first_played {
        stats = stats.push(
            text(format!(
                "First played: {}",
                first.strftime("%Y-%m-%d")
            ))
            .color(MUTED)
            .size(12),
        );
    }

    col = col.push(stats);

    container(scrollable(container(col.padding(l())).center_x(Fill)))
        .height(Fill)
        .into()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
