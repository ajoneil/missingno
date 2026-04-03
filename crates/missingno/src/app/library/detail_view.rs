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
        buttons, fonts, horizontal_rule,
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
    pub save_manifest: Option<library::saves::SaveManifest>,
    pub hovered_log_entry: Option<usize>,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: DetailData<'_>) -> Element<'_, app::Message> {
    let left = game_info_panel(&data);
    let right = activity_log(&data);

    row![left, right]
        .height(Fill)
        .into()
}

/// Left panel: game identity, play button, metadata, actions.
fn game_info_panel<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    let mut col = column![].spacing(m()).width(320);

    // Cover art
    if let Some(handle) = data.cover {
        col = col.push(
            image(handle.clone())
                .width(280)
                .content_fit(iced::ContentFit::ScaleDown),
        );
    }

    // Title
    col = col.push(app_text::heading(data.entry.display_title()));

    // Subtitle
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

    // Play stats summary
    if let Some(log) = &data.play_log {
        if !log.sessions.is_empty() {
            let play_time = log.format_play_time();
            col = col.push(app_text::detail(format!("{play_time} played")).color(MUTED));
        }
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

    // Management
    col = col.push(horizontal_rule());
    col = col.push(
        column![
            buttons::subtle("Import Save...").on_press(app::Message::ImportSave).width(Fill),
            buttons::subtle("Open Folder").on_press(app::Message::OpenGameFolder).width(Fill),
            buttons::subtle("Refresh Metadata").on_press(app::Message::RefreshMetadata).width(Fill),
            buttons::danger("Remove Game").on_press(app::Message::RemoveGame).width(Fill),
        ]
        .spacing(s()),
    );

    scrollable(col.padding(l()))
        .height(Fill)
        .into()
}

/// Right panel: chronological activity log — sessions, saves, imports.
fn activity_log<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    let mut log = column![
        app_text::label("Activity"),
    ]
    .spacing(m())
    .width(Fill);

    let play_log = data.play_log.as_ref();
    let manifest = data.save_manifest.as_ref();

    // Build a unified timeline of events, newest first
    let mut events: Vec<LogEvent> = Vec::new();

    // Add sessions
    if let Some(pl) = play_log {
        for (idx, session) in pl.sessions.iter().enumerate() {
            events.push(LogEvent::Session {
                index: idx,
                start: session.start,
                end: session.end,
            });
        }
    }

    // Add imported saves (not tied to sessions)
    if let Some(m) = manifest {
        for save in &m.saves {
            if matches!(
                save.origin,
                library::saves::SaveOrigin::Imported | library::saves::SaveOrigin::LegacyImport
            ) {
                events.push(LogEvent::Import {
                    save_id: save.id.clone(),
                    timestamp: save.created,
                    size_bytes: save.size_bytes,
                });
            }
        }
    }

    // Sort by timestamp, newest first
    events.sort_by(|a, b| b.timestamp().cmp(&a.timestamp()));

    if events.is_empty() {
        log = log.push(app_text::detail("No activity yet").color(MUTED));
    }

    let hovered = data.hovered_log_entry;

    for (event_idx, event) in events.iter().enumerate() {
        let is_hovered = hovered == Some(event_idx);

        let card = match event {
            LogEvent::Session { index, start, end } => {
                let session_saves: Vec<&library::saves::SaveEntry> = manifest
                    .map(|m| {
                        m.saves
                            .iter()
                            .filter(|s| s.session_index == Some(*index))
                            .collect()
                    })
                    .unwrap_or_default();

                session_entry(*start, *end, &session_saves, is_hovered)
            }
            LogEvent::Import {
                save_id,
                timestamp,
                size_bytes,
            } => import_entry(save_id, *timestamp, *size_bytes, is_hovered),
        };

        log = log.push(
            mouse_area(card)
                .on_enter(app::Message::HoverLogEntry(event_idx))
                .on_exit(app::Message::UnhoverLogEntry),
        );
    }

    scrollable(log.padding(l()))
        .height(Fill)
        .into()
}

fn session_entry<'a>(
    start: jiff::Timestamp,
    end: Option<jiff::Timestamp>,
    saves: &[&library::saves::SaveEntry],
    is_hovered: bool,
) -> Element<'a, app::Message> {
    let detail = if let Some(end) = end {
        let secs = end.duration_since(start).as_secs();
        let mins = secs / 60;
        let hours = mins / 60;
        let duration = if hours > 0 {
            format!("{}h {}m", hours, mins % 60)
        } else if mins > 0 {
            format!("{mins}m")
        } else {
            "< 1m".to_string()
        };
        let start_str = library::saves::format_local(&start);
        let end_time = library::saves::format_local_time(&end);
        format!("{start_str} – {end_time} ({duration})")
    } else {
        let start_str = library::saves::format_local(&start);
        format!("{start_str} – in progress")
    };

    let save_summary = if !saves.is_empty() {
        let n = saves.len();
        let last_time = library::saves::format_local_time(&saves.last().unwrap().created);
        format!(" · {n} save{} · last at {last_time}", if n == 1 { "" } else { "s" })
    } else {
        String::new()
    };

    let mut header = row![
        icons::m(Icon::Play),
        column![
            text("Played").font(fonts::bold()),
            app_text::detail(format!("{detail}{save_summary}")).color(MUTED),
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(s())
    .align_y(Center);

    if is_hovered && !saves.is_empty() {
        let last_id = saves.last().unwrap().id.clone();
        header = header.push(
            row![
                buttons::subtle(app_text::detail("Export"))
                    .on_press(app::Message::ExportSave(last_id.clone())),
                buttons::subtle(app_text::detail("Play from here"))
                    .on_press(app::Message::PlayWithSave(last_id)),
            ]
            .spacing(s()),
        );
    }

    let col = column![header];

    container(col)
        .width(Fill)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border::default().rounded(6),
                ..Default::default()
            }
        })
        .padding(m())
        .into()
}

fn import_entry<'a>(
    save_id: &str,
    timestamp: jiff::Timestamp,
    size_bytes: u32,
    is_hovered: bool,
) -> Element<'a, app::Message> {
    let time = library::saves::format_local(&timestamp);
    let size_kb = size_bytes / 1024;

    let mut content = row![
        icons::m(Icon::Download),
        column![
            text("Save imported").font(fonts::bold()),
            app_text::detail(format!("{time} · {size_kb} KB")).color(MUTED),
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(s())
    .align_y(Center);

    if is_hovered {
        content = content.push(
            row![
                buttons::subtle(app_text::detail("Export"))
                    .on_press(app::Message::ExportSave(save_id.to_string())),
                buttons::subtle(app_text::detail("Play from here"))
                    .on_press(app::Message::PlayWithSave(save_id.to_string())),
            ]
            .spacing(s()),
        );
    }

    container(content)
    .width(Fill)
    .style(|theme: &iced::Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(palette.background.weak.color.into()),
            border: iced::Border::default().rounded(6),
            ..Default::default()
        }
    })
    .padding(m())
    .into()
}

enum LogEvent {
    Session {
        index: usize,
        start: jiff::Timestamp,
        end: Option<jiff::Timestamp>,
    },
    Import {
        save_id: String,
        timestamp: jiff::Timestamp,
        size_bytes: u32,
    },
}

impl LogEvent {
    fn timestamp(&self) -> jiff::Timestamp {
        match self {
            LogEvent::Session { start, .. } => *start,
            LogEvent::Import { timestamp, .. } => *timestamp,
        }
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
