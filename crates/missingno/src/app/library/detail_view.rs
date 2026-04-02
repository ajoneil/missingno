use std::path::PathBuf;

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
        buttons, horizontal_rule,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    library::{self, GameEntry, play_log::PlayLog},
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

pub struct DetailData<'a> {
    pub entry: &'a GameEntry,
    pub cover: Option<&'a image::Handle>,
    pub play_log: Option<PlayLog>,
    pub is_running: bool,
    pub game_dir: Option<PathBuf>,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: DetailData<'_>) -> Element<'_, app::Message> {
    let mut col = column![].spacing(l()).max_width(500);

    // Cover art
    if let Some(handle) = data.cover {
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
    let play_label = if data.is_running { "Resume" } else { "Play" };
    col = col.push(
        container(buttons::primary(play_label).on_press(app::Message::PlayFromDetail))
            .center_x(Fill),
    );

    // Title
    col = col.push(app_text::xl(data.entry.display_title()));

    // Subtitle: publisher · year · platform
    let subtitle_parts: Vec<&str> = [
        data.entry.publisher.as_deref(),
        data.entry.year.as_deref(),
        data.entry.platform.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !subtitle_parts.is_empty() {
        col = col.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Links
    if data.entry.wikipedia_url.is_some() || data.entry.igdb_url.is_some() {
        let mut links = row![].spacing(m());

        if let Some(url) = &data.entry.wikipedia_url {
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

        if let Some(url) = &data.entry.igdb_url {
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
    if let Some(log) = &data.play_log {
        let play_time = log.format_play_time();
        let sessions = log.sessions.len();
        let saves = log.save_events.len();

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
        if let Some(first) = &log.first_played {
            stats = stats.push(
                text(format!("First played: {}", first.strftime("%Y-%m-%d")))
                    .color(MUTED)
                    .size(12),
            );
        }
        col = col.push(stats);
    }

    // Save history
    if let Some(game_dir) = &data.game_dir {
        let saves = library::list_save_history(game_dir);
        let has_battery = library::battery_path(game_dir).exists();

        if has_battery || !saves.is_empty() {
            col = col.push(horizontal_rule());

            let mut save_section = column![app_text::m("Save Data")].spacing(s());

            if has_battery {
                save_section = save_section.push(text("Current save: battery.sav").color(MUTED));
            }

            // Show recent saves (limit to 10)
            if !saves.is_empty() {
                save_section = save_section.push(text("History:").color(MUTED).size(12));
                for (name, path) in saves.into_iter().take(10) {
                    save_section = save_section.push(
                        row![
                            text(name).color(MUTED).size(12).width(Fill),
                            buttons::subtle("Restore")
                                .on_press(app::Message::RestoreSave(path)),
                        ]
                        .spacing(s())
                        .align_y(Center),
                    );
                }
            }

            save_section = save_section.push(
                buttons::standard("Import Save...")
                    .on_press(app::Message::ImportSave),
            );

            col = col.push(save_section);
        }
    }

    // Management actions
    col = col.push(horizontal_rule());
    col = col.push(
        row![
            buttons::subtle("Open Folder").on_press(app::Message::OpenGameFolder),
            buttons::subtle("Refresh Metadata").on_press(app::Message::RefreshMetadata),
            buttons::danger("Remove").on_press(app::Message::RemoveGame),
        ]
        .spacing(s()),
    );

    container(scrollable(container(col.padding(l())).center_x(Fill)))
        .height(Fill)
        .into()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
