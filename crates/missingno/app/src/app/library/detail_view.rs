use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    mouse,
    widget::{button, column, container, image, mouse_area, row, scrollable, text},
};

use crate::app::{
    self, launch,
    library::{
        GameEntry,
        activity::{self, ActivityKind, SessionFile},
        store::{ActivityState, SessionSummary},
    },
    ui::{
        buttons, containers, fonts, horizontal_rule,
        icons::{self, Icon},
        palette::MUTED,
        sizes::{border_s, l, m, s},
        text as app_text,
    },
};
use crate::cartridge_rw;

const COVER_HEIGHT: f32 = 160.0;
const COVER_WIDTH: f32 = 120.0;

/// The bodies a game's details page switches between, under its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Activity,
    GameSettings,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Activity => "Activity",
            Section::GameSettings => "Game Settings",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Section::Activity => Icon::Play,
            Section::GameSettings => Icon::Sliders,
        }
    }
}

const SECTIONS: [Section; 2] = [Section::Activity, Section::GameSettings];
const RAIL_WIDTH: f32 = 220.0;

pub struct DetailData<'a> {
    pub entry: &'a GameEntry,
    pub cover: Option<&'a image::Handle>,
    pub activity_state: &'a ActivityState,
    pub live_session: Option<&'a SessionFile>,
    pub live_screenshots: &'a [image::Handle],
    pub live_prints: &'a [image::Handle],
    pub section: Section,
    pub hovered_log_entry: Option<usize>,
    pub header_hovered: bool,
    /// Whether this game is currently loaded and running.
    pub is_loaded: bool,
    /// The inserted cartridge, if any, for flash writing.
    pub inserted_cartridge: Option<&'a cartridge_rw::CartridgeHeader>,
    /// The game's launch options, absent where no family claims its platform.
    pub launch_options: Option<launch::PanelData>,
}

#[allow(private_interfaces)]
pub(crate) fn view(data: DetailData<'_>) -> Element<'_, app::Message> {
    let body = match data.section {
        Section::GameSettings => game_settings(&data),
        Section::Activity => match data.activity_state {
            ActivityState::Loading => activity_loading(),
            ActivityState::Loaded(detail) => activity_log(
                &detail.sessions,
                data.live_session,
                data.live_screenshots,
                data.live_prints,
                data.hovered_log_entry,
            ),
        },
    };

    column![game_header(&data), row![rail(data.section), body]]
        .height(Fill)
        .into()
}

/// Which body the header stands over.
fn rail(current: Section) -> Element<'static, app::Message> {
    let mut col = column![].spacing(s());
    for section in SECTIONS {
        let label = row![icons::m(section.icon()), text(section.label())]
            .spacing(s())
            .align_y(Center);
        col = col.push(if section == current {
            buttons::selected(label).width(Fill)
        } else {
            buttons::subtle(label)
                .on_press(app::Message::Detail(app::DetailMessage::SelectSection(
                    section,
                )))
                .width(Fill)
        });
    }

    container(col.padding(m()))
        .width(RAIL_WIDTH)
        .height(Fill)
        .style(containers::sidebar)
        .into()
}

/// Unified header: back + cover + identity + play + settings.
fn game_header<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    use iced::widget::stack;

    let has_rom = data.entry.rom_paths.iter().any(|path| path.exists());

    let back_button = || {
        container(app::automation::tag(
            app::automation::ids::DETAIL_BACK,
            button(icons::m(Icon::Back).style(|_, _| iced::widget::svg::Style {
                color: Some(iced::Color::WHITE),
            }))
            .on_press(app::Message::BackToLibrary)
            .style(|_, status| {
                let bg_alpha = match status {
                    button::Status::Hovered => 0.9,
                    _ => 0.7,
                };
                button::Style {
                    background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into()),
                    text_color: iced::Color::WHITE,
                    border: iced::Border::default().rounded(border_s()),
                    ..Default::default()
                }
            }),
        ))
        .padding([m() + 4.0, m()])
    };

    // Cover thumbnail with back button overlay — clickable to play if ROM exists
    let cover: Element<'_, app::Message> = if let Some(handle) = data.cover {
        let cover_img: Element<'_, app::Message> = image(handle.clone())
            .height(COVER_HEIGHT)
            .content_fit(iced::ContentFit::ScaleDown)
            .into();

        let cover_el: Element<'_, app::Message> = if data.header_hovered {
            stack![cover_img, back_button()].into()
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
        let placeholder = super::view::cartridge_placeholder(
            &data.entry.display_title(),
            data.entry.platform,
            COVER_WIDTH,
            COVER_HEIGHT,
            iced::border::Radius::from(0.0),
        );

        stack![placeholder, back_button()].into()
    };

    let mut info = column![
        app_text::heading(data.entry.display_title()).wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(4);

    let subtitle_parts: Vec<String> = [
        data.entry.publisher.clone(),
        data.entry
            .year
            .as_ref()
            .map(|year| activity::release_year(year)),
        data.entry
            .platform
            .map(|platform| platform.name().to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !subtitle_parts.is_empty() {
        info = info.push(text(subtitle_parts.join(" · ")).color(MUTED));
    }

    let mut meta_parts = row![].spacing(m()).align_y(Center);
    let mut has_meta = false;

    if let ActivityState::Loaded(detail) = data.activity_state {
        let total_secs: f64 = detail
            .sessions
            .iter()
            .filter(|entry| entry.kind == ActivityKind::Session)
            .filter_map(|entry| {
                entry
                    .end
                    .map(|end: jiff::Timestamp| end.duration_since(entry.start).as_secs_f64())
            })
            .sum();
        if total_secs > 0.0 {
            meta_parts = meta_parts
                .push(app_text::detail(activity::format_play_time(total_secs)).color(MUTED));
            has_meta = true;
        }
    }

    for (url, label) in [
        (&data.entry.wikipedia_url, "Wikipedia"),
        (&data.entry.igdb_url, "IGDB"),
    ] {
        if let Some(url) = url {
            meta_parts = meta_parts.push(
                mouse_area(
                    row![icons::m(Icon::Globe), text(label).color(MUTED)]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::OpenUrl(leak_str(url)))
                .interaction(mouse::Interaction::Pointer),
            );
            has_meta = true;
        }
    }

    if has_meta {
        info = info.push(meta_parts);
    }

    let mut actions = row![].spacing(s()).align_y(Center);
    if has_rom {
        let label = if data.is_loaded { "Resume" } else { "Play" };
        actions = actions.push(app::automation::tag(
            app::automation::ids::DETAIL_PLAY,
            buttons::primary(
                row![icons::m(Icon::Play), label]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(app::Message::PlayFromDetail),
        ));
        if data.is_loaded {
            actions = actions.push(app::automation::tag(
                app::automation::ids::DETAIL_STOP,
                buttons::danger("Stop").on_press(app::Message::StopGame),
            ));
        }
    }
    // Cartridge actions button — only show when the cart matches this game or is flashable
    if let Some(cart) = data.inserted_cartridge {
        let cart_matches = data
            .entry
            .header_title
            .as_ref()
            .is_some_and(|title| title == &cart.title);
        if cart_matches || cart.flashable() {
            actions = actions.push(app::automation::tag(
                app::automation::ids::DETAIL_CARTRIDGE,
                buttons::standard(
                    row![icons::m(Icon::CircuitBoard), "Cartridge"]
                        .spacing(s())
                        .align_y(Center),
                )
                .on_press(app::Message::Cartridge(
                    app::CartridgeMessage::ShowActions(data.entry.sha1.clone()),
                )),
            ));
        }
    }
    actions = actions.push(app::automation::tag(
        app::automation::ids::DETAIL_MENU,
        buttons::subtle(icons::m(Icon::Menu)).on_press(app::Message::ToggleMenu),
    ));

    let header = row![
        cover,
        container(row![info.width(Fill), actions].spacing(m()))
            .padding([m() + 4.0, m()])
            .width(Fill)
            .height(COVER_HEIGHT),
    ];

    let header = mouse_area(header)
        .on_enter(app::Message::Detail(app::DetailMessage::HoverHeader))
        .on_exit(app::Message::Detail(app::DetailMessage::UnhoverHeader));

    column![header, horizontal_rule()].into()
}

/// The same launch panel the window shows, over the game's stored overrides.
fn game_settings<'a>(data: &DetailData<'a>) -> Element<'a, app::Message> {
    let body: Element<'_, app::Message> = match &data.launch_options {
        Some(options) => launch::panel(options),
        None => app_text::detail("No system is registered for this game.")
            .color(MUTED)
            .into(),
    };

    scrollable(container(body).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

fn activity_loading() -> Element<'static, app::Message> {
    container(app_text::detail("Loading…").color(MUTED))
        .padding(l())
        .width(Fill)
        .into()
}

/// Right panel: chronological activity log.
fn activity_log<'a>(
    sessions: &'a [SessionSummary],
    live_session: Option<&SessionFile>,
    live_screenshots: &'a [image::Handle],
    live_prints: &'a [image::Handle],
    hovered_log_entry: Option<usize>,
) -> Element<'a, app::Message> {
    let mut log = column![].spacing(m()).width(Fill);

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
            prints: live_prints.to_vec(),
            size_bytes: None,
            import_source: None,
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
                .on_enter(app::Message::Detail(app::DetailMessage::HoverLogEntry(idx)))
                .on_exit(app::Message::Detail(app::DetailMessage::UnhoverLogEntry)),
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
        ActivityKind::CartridgeWrite => cart_write_card(entry),
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
                    buttons::subtle(app_text::detail("Export")).on_press(app::Message::Detail(
                        app::DetailMessage::ExportSave(entry.filename.clone())
                    )),
                    buttons::subtle(app_text::detail("Play from here")).on_press(
                        app::Message::Detail(app::DetailMessage::PlayWithSave(
                            entry.filename.clone()
                        ))
                    ),
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
                button(image(handle.clone()).width(160).height(144))
                    .on_press(app::Message::Detail(
                        app::DetailMessage::OpenScreenshotGallery(filename.clone(), i),
                    ))
                    .padding(0)
                    .style(|_, _| button::Style::default()),
            );
        }
        if total > max_visible {
            let remaining = total - max_visible;
            thumb_row = thumb_row.push(
                button(
                    container(text(format!("+{remaining}")).size(20.0).color(MUTED))
                        .width(80)
                        .height(144)
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .style(|theme: &iced::Theme| {
                            let palette = theme.extended_palette();
                            container::Style {
                                background: Some(palette.background.strong.color.into()),
                                border: iced::Border::default().rounded(border_s()),
                                ..Default::default()
                            }
                        }),
                )
                .on_press(app::Message::Detail(
                    app::DetailMessage::OpenScreenshotGallery(filename.clone(), max_visible),
                ))
                .padding(0)
                .style(|_, _| button::Style::default()),
            );
        }
        card = card.push(thumb_row);
    }

    if !entry.prints.is_empty() {
        let max_visible = 4;
        let total = entry.prints.len();
        let mut print_row = row![].spacing(s()).align_y(iced::alignment::Vertical::Top);
        for handle in entry.prints.iter().take(max_visible) {
            print_row = print_row.push(
                container(image(handle.clone()).width(120))
                    .padding(s())
                    .style(containers::card),
            );
        }
        if total > max_visible {
            print_row = print_row.push(
                text(format!("+{}", total - max_visible))
                    .size(20.0)
                    .color(MUTED),
            );
        }
        card = card.push(column![app_text::detail("Prints").color(MUTED), print_row].spacing(s()));
    }

    container(card)
        .width(Fill)
        .style(containers::card)
        .padding(m())
        .into()
}

fn import_card(entry: &SessionSummary, is_hovered: bool) -> Element<'static, app::Message> {
    let time = activity::format_local(&entry.start);
    let size_kb = entry.size_bytes.unwrap_or(0) / 1024;

    let from_cartridge = matches!(
        entry.import_source,
        Some(activity::ImportSource::Cartridge { .. })
    );
    let (icon, label) = if from_cartridge {
        (Icon::CircuitBoard, "Save imported from cartridge")
    } else {
        (Icon::Download, "Save imported")
    };

    let mut content = row![
        icons::m(icon),
        column![
            text(label).font(fonts::bold()),
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
                buttons::subtle(app_text::detail("Export")).on_press(app::Message::Detail(
                    app::DetailMessage::ExportSave(entry.filename.clone())
                )),
                buttons::subtle(app_text::detail("Play from here")).on_press(app::Message::Detail(
                    app::DetailMessage::PlayWithSave(entry.filename.clone())
                )),
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
        .style(containers::card)
        .padding(m())
        .into()
}

fn cart_write_card(entry: &SessionSummary) -> Element<'static, app::Message> {
    let time = activity::format_local(&entry.start);
    let size_kb = entry.size_bytes.unwrap_or(0) / 1024;

    let content = row![
        icons::m(Icon::CircuitBoard),
        column![
            text("Save written to cartridge").font(fonts::bold()),
            app_text::detail(format!("{time} · {size_kb} KB")).color(MUTED),
        ]
        .spacing(2)
        .width(Fill),
    ]
    .spacing(s())
    .align_y(Center);

    container(content)
        .width(Fill)
        .style(containers::card)
        .padding(m())
        .into()
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}
