use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{button, column, container, image, mouse_area, row, scrollable, stack, text},
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
    pub cover_hovered: bool,
    pub window_height: f32,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: DetailData<'_>) -> Element<'_, app::Message> {
    let left = game_info_panel(&data);
    let right = activity_log(&data);

    row![left, right].height(Fill).into()
}

/// Left panel: game identity, play button, metadata, actions.
fn game_info_panel<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    let has_rom = data.entry.rom_paths.iter().any(|p| p.exists());
    let cover_hovered = data.cover_hovered;

    let cover: Element<'_, app::Message> = if let Some(handle) = data.cover {
        // Cover height capped so info/actions always fit.
        // 450px accounts for action bar, title, metadata, buttons, padding.
        let max_cover_h = (data.window_height - 450.0).max(80.0);

        let cover_img = container(
            image(handle.clone())
                .content_fit(iced::ContentFit::ScaleDown)
                .border_radius(8),
        )
        .max_height(max_cover_h);

        if has_rom {
            let cover_el: Element<'_, app::Message> = if cover_hovered {
                container(stack![
                    cover_img,
                    iced::widget::opaque(
                        container(iced::widget::Space::new())
                            .width(Fill)
                            .height(Fill)
                            .style(|_: &iced::Theme| container::Style {
                                background: Some(
                                    iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4).into(),
                                ),
                                border: iced::Border::default().rounded(8),
                                ..Default::default()
                            }),
                    ),
                    container(
                        button(
                            icons::xl(Icon::Play).style(|_, _| iced::widget::svg::Style {
                                color: Some(Color::WHITE),
                            }),
                        )
                        .on_press(app::Message::PlayFromDetail)
                        .style(|_: &iced::Theme, status| {
                            let bg_alpha = match status {
                                button::Status::Hovered => 0.8,
                                _ => 0.5,
                            };
                            button::Style {
                                background: Some(
                                    iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into(),
                                ),
                                text_color: Color::WHITE,
                                border: iced::Border::default().rounded(24),
                                ..Default::default()
                            }
                        }),
                    )
                    .width(Fill)
                    .height(Fill)
                    .align_x(Center)
                    .align_y(iced::alignment::Vertical::Center)
                ])
                .center_x(Fill)
                .into()
            } else {
                container(cover_img).center_x(Fill).into()
            };

            mouse_area(cover_el)
                .on_enter(app::Message::HoverCover)
                .on_exit(app::Message::UnhoverCover)
                .interaction(mouse::Interaction::Pointer)
                .into()
        } else {
            cover_img.into()
        }
    } else {
        iced::widget::Space::new().into()
    };

    // Fixed-height info
    let mut info = column![app_text::heading(data.entry.display_title())].spacing(4);

    let subtitle_parts: Vec<&str> = [
        data.entry.publisher.as_deref(),
        data.entry.year.as_deref(),
        data.entry.platform.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    if let Some(log) = &data.play_log {
        if !log.sessions.is_empty() {
            info = info.push(
                app_text::detail(format!("{} played", log.format_play_time())).color(MUTED),
            );
        }
    }

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
        info = info.push(links);
    }

    // Fixed-height actions
    let actions = column![
        horizontal_rule(),
        column![
            buttons::subtle("Import Save...")
                .on_press(app::Message::ImportSave)
                .width(Fill),
            buttons::subtle("Open Folder")
                .on_press(app::Message::OpenGameFolder)
                .width(Fill),
            buttons::subtle("Refresh Metadata")
                .on_press(app::Message::RefreshMetadata)
                .width(Fill),
            buttons::danger("Remove Game")
                .on_press(app::Message::RemoveGame)
                .width(Fill),
        ]
        .spacing(s()),
    ]
    .spacing(m());

    column![cover, info, actions]
        .spacing(m())
        .padding(l())
        .width(400)
        .into()
}

/// Right panel: chronological activity log — sessions, saves, imports.
fn activity_log<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    let mut log = column![app_text::label("Activity"),]
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

    scrollable(log.padding(l())).height(Fill).into()
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

    let mut info_col = column![
        text("Played").font(fonts::bold()),
        app_text::detail(detail).color(MUTED),
    ]
    .spacing(2);

    if !saves.is_empty() {
        let n = saves.len();
        let last_time = library::saves::format_local_time(&saves.last().unwrap().created);
        info_col = info_col.push(
            app_text::detail(format!(
                "{n} save{} · last at {last_time}",
                if n == 1 { "" } else { "s" }
            ))
            .color(MUTED),
        );
    }

    let mut header = row![icons::m(Icon::Play), info_col.width(Fill),]
        .spacing(s())
        .align_y(Center);

    if !saves.is_empty() {
        let last_id = saves.last().unwrap().id.clone();
        if is_hovered {
            header = header.push(
                row![
                    buttons::subtle(app_text::detail("Export"))
                        .on_press(app::Message::ExportSave(last_id.clone())),
                    buttons::subtle(app_text::detail("Play from here"))
                        .on_press(app::Message::PlayWithSave(last_id)),
                ]
                .spacing(s()),
            );
        } else {
            header = header.push(
                row![
                    buttons::invisible(app_text::detail("Export")),
                    buttons::invisible(app_text::detail("Play from here")),
                ]
                .spacing(s()),
            );
        }
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
    } else {
        content = content.push(
            row![
                buttons::invisible(app_text::detail("Export")),
                buttons::invisible(app_text::detail("Play from here")),
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
