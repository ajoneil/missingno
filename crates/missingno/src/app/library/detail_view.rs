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
    pub is_running: bool,
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
    col = col.push(app_text::heading(data.entry.display_title()));

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
        let sessions = log.sessions.len();
        if sessions > 0 {
            let play_time = log.format_play_time();
            let saves = data.save_manifest.as_ref().map(|m| m.saves.len()).unwrap_or(0);

            let mut stats = column![].spacing(s());
            stats = stats.push(text(format!("Play time: {play_time}")).color(MUTED));
            stats = stats.push(
                text(format!(
                    "{sessions} session{}, {saves} save{}",
                    if sessions == 1 { "" } else { "s" },
                    if saves == 1 { "" } else { "s" },
                ))
                .color(MUTED),
            );
            if let Some(first) = &log.first_played {
                stats = stats.push(
                    app_text::detail(format!("First played: {}", first.strftime("%Y-%m-%d")))
                        .color(MUTED),
                );
            }
            col = col.push(stats);
        }
    }

    // Save timeline — grouped by session
    if let Some(manifest) = &data.save_manifest {
        let has_saves = !manifest.saves.is_empty();
        let has_sessions = data.play_log.as_ref().map(|l| !l.sessions.is_empty()).unwrap_or(false);

        if has_saves || has_sessions {
            col = col.push(horizontal_rule());

            let mut save_section = column![app_text::label("Save Timeline")].spacing(m());

            let boot_save = manifest.current.as_deref()
                .or_else(|| manifest.saves.last().map(|s| s.id.as_str()));

            // Build session groups (newest first)
            if let Some(log) = &data.play_log {
                for (idx, session) in log.sessions.iter().enumerate().rev().take(10) {
                    let session_start = library::saves::format_local(&session.start);
                    let session_label = if let Some(end) = &session.end {
                        let duration_secs = end.duration_since(session.start).as_secs();
                        let mins = duration_secs / 60;
                        if mins > 0 {
                            format!("{session_start} · {mins}m")
                        } else {
                            session_start
                        }
                    } else {
                        format!("{session_start} · in progress")
                    };

                    // Find saves for this session
                    let session_saves: Vec<&library::saves::SaveEntry> = manifest
                        .saves
                        .iter()
                        .filter(|s| s.session_index == Some(idx))
                        .collect();

                    let mut session_col = column![
                        app_text::detail(session_label).font(fonts::bold()),
                    ]
                    .spacing(s());

                    if session_saves.is_empty() {
                        session_col = session_col.push(
                            app_text::detail("No saves").color(MUTED),
                        );
                    } else {
                        for save in &session_saves {
                            let is_boot = boot_save == Some(save.id.as_str());
                            let time = library::saves::format_local_time(&save.created);
                            let size_kb = save.size_bytes / 1024;

                            let label = if is_boot {
                                format!("{time} · {size_kb} KB ●")
                            } else {
                                format!("{time} · {size_kb} KB")
                            };

                            session_col = session_col.push(
                                buttons::subtle(app_text::detail(label))
                                    .on_press(app::Message::SelectBootSave(save.id.clone()))
                                    .width(Fill),
                            );
                        }
                    }

                    save_section = save_section.push(session_col);
                }
            }

            // Imported saves (no session)
            let imported: Vec<&library::saves::SaveEntry> = manifest
                .saves
                .iter()
                .filter(|s| matches!(s.origin, library::saves::SaveOrigin::Imported | library::saves::SaveOrigin::LegacyImport))
                .collect();

            if !imported.is_empty() {
                let mut import_col = column![
                    app_text::detail("Imported").font(fonts::bold()),
                ]
                .spacing(s());

                for save in &imported {
                    let is_boot = boot_save == Some(save.id.as_str());
                    let time = library::saves::format_local(&save.created);
                    let size_kb = save.size_bytes / 1024;
                    let label = if is_boot {
                        format!("{time} · {size_kb} KB ●")
                    } else {
                        format!("{time} · {size_kb} KB")
                    };

                    import_col = import_col.push(
                        buttons::subtle(app_text::detail(label))
                            .on_press(app::Message::SelectBootSave(save.id.clone()))
                            .width(Fill),
                    );
                }

                save_section = save_section.push(import_col);
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
