use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    mouse,
    widget::{button, column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self,
    core::{
        buttons, fonts, horizontal_rule,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    library::{
        GameEntry,
        activity::{self, ActivityKind, SessionFile},
        store::{ActivityState, SessionSummary},
    },
};

const COVER_HEIGHT: f32 = 160.0;
const COVER_WIDTH: f32 = 120.0;

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

pub struct DetailData<'a> {
    pub entry: &'a GameEntry,
    pub cover: Option<&'a image::Handle>,
    pub activity_state: &'a ActivityState,
    pub live_session: Option<&'a SessionFile>,
    pub live_screenshots: &'a [image::Handle],
    pub hovered_log_entry: Option<usize>,
    pub header_hovered: bool,
    /// Whether this game is currently loaded and running.
    pub is_loaded: bool,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: DetailData<'_>) -> Element<'_, app::Message> {
    let header = game_header(&data);
    let content = match data.activity_state {
        ActivityState::Loading => activity_loading(),
        ActivityState::Loaded(detail) => activity_log(
            &detail.sessions,
            data.live_session,
            data.live_screenshots,
            data.hovered_log_entry,
        ),
    };

    column![header, content].height(Fill).into()
}

/// Unified header: back + cover + identity + play + settings.
fn game_header<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    use iced::widget::stack;

    let has_rom = data.entry.rom_paths.iter().any(|p| p.exists());

    // Cover thumbnail with back button overlay — clickable to play if ROM exists
    let cover: Element<'_, app::Message> = if let Some(handle) = data.cover {
        let cover_img: Element<'_, app::Message> = image(handle.clone())
            .height(COVER_HEIGHT)
            .content_fit(iced::ContentFit::ScaleDown)
            .into();

        let cover_el: Element<'_, app::Message> = if data.header_hovered {
            let back_btn = container(
                button(
                    icons::m(Icon::Back).style(|_, _| iced::widget::svg::Style {
                        color: Some(iced::Color::WHITE),
                    }),
                )
                .on_press(app::Message::BackToLibrary)
                .style(|_, status| {
                    let bg_alpha = match status {
                        button::Status::Hovered => 0.9,
                        _ => 0.7,
                    };
                    button::Style {
                        background: Some(
                            iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into(),
                        ),
                        text_color: iced::Color::WHITE,
                        border: iced::Border::default().rounded(4),
                        ..Default::default()
                    }
                }),
            )
            .padding([m() + 4.0, m()]);

            stack![cover_img, back_btn].into()
        } else {
            cover_img
        };

        if has_rom {
            mouse_area(cover_el)
                .on_press(app::Message::PlayFromDetail)
                .interaction(mouse::Interaction::Pointer)
                .into()
        } else {
            cover_el
        }
    } else {
        container(
            buttons::subtle(icons::m(Icon::Back)).on_press(app::Message::BackToLibrary),
        )
        .width(COVER_WIDTH)
        .height(COVER_HEIGHT)
        .into()
    };

    // Title + metadata column
    let mut info = column![app_text::heading(data.entry.display_title())].spacing(4);

    let subtitle_parts: Vec<String> = [
        data.entry.publisher.clone(),
        data.entry.year.as_ref().map(|y| activity::format_date_string(y)),
        data.entry.platform.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    // Play time + links on one line
    let mut meta_parts = row![].spacing(m()).align_y(Center);
    let mut has_meta = false;

    if let ActivityState::Loaded(detail) = data.activity_state {
        let total_secs: f64 = detail
            .sessions
            .iter()
            .filter(|a| a.kind == ActivityKind::Session)
            .filter_map(|a| {
                a.end
                    .map(|end: jiff::Timestamp| end.duration_since(a.start).as_secs_f64())
            })
            .sum();
        if total_secs > 0.0 {
            meta_parts = meta_parts
                .push(app_text::detail(activity::format_play_time(total_secs)).color(MUTED));
            has_meta = true;
        }
    }

    if let Some(url) = &data.entry.wikipedia_url {
        meta_parts = meta_parts.push(
            mouse_area(
                row![icons::m(Icon::Globe), text("Wikipedia").color(MUTED)]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::OpenUrl(leak_str(url)))
            .interaction(mouse::Interaction::Pointer),
        );
        has_meta = true;
    }
    if let Some(url) = &data.entry.igdb_url {
        meta_parts = meta_parts.push(
            mouse_area(
                row![icons::m(Icon::Globe), text("IGDB").color(MUTED)]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::OpenUrl(leak_str(url)))
            .interaction(mouse::Interaction::Pointer),
        );
        has_meta = true;
    }

    if has_meta {
        info = info.push(meta_parts);
    }

    // Right side: primary row on top, secondary row below on hover
    let mut primary = row![].spacing(s()).align_y(Center);
    if has_rom {
        if data.is_loaded {
            primary = primary.push(
                buttons::primary(
                    row![icons::m(Icon::Play), "Resume"]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::PlayFromDetail),
            );
            primary = primary.push(buttons::danger("Stop").on_press(app::Message::StopGame));
        } else {
            primary = primary.push(
                buttons::primary(
                    row![icons::m(Icon::Play), "Play"]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::PlayFromDetail),
            );
        }
    }
    primary = primary.push(
        buttons::subtle(
            row![icons::m(Icon::Gear), "Settings"]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(app::Message::ShowSettings),
    );

    let mut right = column![primary]
        .align_x(iced::alignment::Horizontal::Right);

    if data.header_hovered {
        right = right.push(iced::widget::Space::new().height(Fill));
        right = right.push(
            row![
                buttons::subtle(app_text::detail("Import Save..."))
                    .on_press(app::Message::ImportSave),
                buttons::subtle(app_text::detail("Open Folder"))
                    .on_press(app::Message::OpenGameFolder),
                buttons::subtle(app_text::detail("Refresh"))
                    .on_press(app::Message::RefreshMetadata),
                buttons::danger(app_text::detail("Remove"))
                    .on_press(app::Message::RemoveGame),
            ]
            .spacing(s()),
        );
    }

    let header = row![
        cover,
        container(
            row![info.width(Fill), right].spacing(m()),
        )
        .padding([m() + 4.0, m()])
        .width(Fill)
        .height(COVER_HEIGHT),
    ];

    let header = mouse_area(header)
        .on_enter(app::Message::HoverHeader)
        .on_exit(app::Message::UnhoverHeader);

    column![header, horizontal_rule()].into()
}

fn activity_loading() -> Element<'static, app::Message> {
    container(
        column![
            app_text::label("Activity"),
            app_text::detail("Loading…").color(MUTED),
        ]
        .spacing(m()),
    )
    .padding(l())
    .width(Fill)
    .into()
}

/// Right panel: chronological activity log.
fn activity_log<'a>(
    sessions: &'a [SessionSummary],
    live_session: Option<&SessionFile>,
    live_screenshots: &'a [image::Handle],
    hovered_log_entry: Option<usize>,
) -> Element<'a, app::Message> {
    let mut log = column![app_text::label("Activity")].spacing(m()).width(Fill);

    // Show live session at the top if one is in progress
    if let Some(live) = live_session {
        let live_summary = SessionSummary {
            filename: String::new(),
            kind: ActivityKind::Session,
            start: live.start,
            end: live.end,
            save_count: live.save_count(),
            last_save_time: live.last_save_time(),
            screenshots: live_screenshots.to_vec(),
            size_bytes: None,
        };
        log = log.push(session_card(&live_summary, false));
    }

    // Filter out the live session from the persisted list to avoid showing it twice.
    let live_start = live_session.map(|s| s.start);

    let filtered: Vec<_> = sessions
        .iter()
        .filter(|s| !(s.kind == ActivityKind::Session && live_start == Some(s.start)))
        .collect();

    if filtered.is_empty() && live_session.is_none() {
        log = log.push(app_text::detail("No activity yet").color(MUTED));
    }

    let hovered = hovered_log_entry;

    for (idx, entry) in filtered.iter().enumerate() {
        let is_hovered = hovered == Some(idx);
        log = log.push(
            mouse_area(activity_card(entry, is_hovered))
                .on_enter(app::Message::HoverLogEntry(idx))
                .on_exit(app::Message::UnhoverLogEntry),
        );
    }

    scrollable(container(log.max_width(1200)).padding(l()).center_x(Fill))
        .height(Fill)
        .into()
}

fn activity_card(entry: &SessionSummary, is_hovered: bool) -> Element<'static, app::Message> {
    match entry.kind {
        ActivityKind::Session => session_card(entry, is_hovered),
        ActivityKind::Import => import_card(entry, is_hovered),
    }
}

fn session_card(entry: &SessionSummary, is_hovered: bool) -> Element<'static, app::Message> {
    let start = entry.start;
    let detail = if let Some(end) = entry.end {
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
        let start_str = activity::format_local(&start);
        let end_time = activity::format_local_time(&end);
        format!("{start_str} – {end_time} ({duration})")
    } else {
        // No end time — either live (shown separately) or interrupted
        activity::format_local(&start)
    };

    let mut info_col = column![
        text("Played").font(fonts::bold()),
        app_text::detail(detail).color(MUTED),
    ]
    .spacing(2);

    if entry.save_count > 0 {
        let n = entry.save_count;
        let last_time = entry
            .last_save_time
            .map(|t| activity::format_local_time(&t))
            .unwrap_or_default();
        info_col = info_col.push(
            app_text::detail(format!(
                "{n} save{} · last at {last_time}",
                if n == 1 { "" } else { "s" }
            ))
            .color(MUTED),
        );
    }

    let mut header = row![icons::m(Icon::Play), info_col.width(Fill)]
        .spacing(s())
        .align_y(Center);

    let has_saves = entry.save_count > 0 && !entry.filename.is_empty();
    if has_saves {
        if is_hovered {
            header = header.push(
                row![
                    buttons::subtle(app_text::detail("Export"))
                        .on_press(app::Message::ExportSave(entry.filename.clone())),
                    buttons::subtle(app_text::detail("Play from here"))
                        .on_press(app::Message::PlayWithSave(entry.filename.clone())),
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

    let mut card = column![header].spacing(s());

    if !entry.screenshots.is_empty() && !entry.filename.is_empty() {
        let filename = entry.filename.clone();
        let max_visible = 4;
        let total = entry.screenshots.len();
        let mut thumb_row = row![].spacing(s());
        for (i, handle) in entry.screenshots.iter().take(max_visible).enumerate() {
            thumb_row = thumb_row.push(
                button(
                    image(handle.clone())
                        .width(160)
                        .height(144),
                )
                .on_press(app::Message::OpenScreenshotGallery(filename.clone(), i))
                .padding(0)
                .style(|_, _| button::Style::default()),
            );
        }
        if total > max_visible {
            let remaining = total - max_visible;
            thumb_row = thumb_row.push(
                button(
                    container(
                        text(format!("+{remaining}"))
                            .size(20.0)
                            .color(MUTED),
                    )
                    .width(80)
                    .height(144)
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Center)
                    .style(|theme: &iced::Theme| {
                        let palette = theme.extended_palette();
                        container::Style {
                            background: Some(palette.background.strong.color.into()),
                            border: iced::Border::default().rounded(4),
                            ..Default::default()
                        }
                    }),
                )
                .on_press(app::Message::OpenScreenshotGallery(filename.clone(), max_visible))
                .padding(0)
                .style(|_, _| button::Style::default()),
            );
        }
        card = card.push(thumb_row);
    }

    container(card)
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

fn import_card(entry: &SessionSummary, is_hovered: bool) -> Element<'static, app::Message> {
    let time = activity::format_local(&entry.start);
    let size_kb = entry.size_bytes.unwrap_or(0) / 1024;

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
                    .on_press(app::Message::ExportSave(entry.filename.clone())),
                buttons::subtle(app_text::detail("Play from here"))
                    .on_press(app::Message::PlayWithSave(entry.filename.clone())),
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

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
