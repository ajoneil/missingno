//! The launch panel and the window that stands it over the screen. Rows are
//! rendered from the family's descriptors alone, so nothing here names an
//! option.

use iced::{
    Alignment,
    Alignment::Center,
    Element,
    widget::{center, column, container, mouse_area, opaque, pick_list, row, scrollable, toggler},
};
use missingno_core::cartridge::{
    AttributeKind, AttributeSpec, AttributeValue, BoardSpec, BoardValue,
};
use missingno_core::launch::{LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues};

use super::{Edit, EditSurface, Facts, Message, Window};
use crate::app;
use crate::app::system::{Platform, platforms_by_name};
use crate::app::ui::{
    buttons, containers, horizontal_rule,
    palette::{MUTED, RED},
    sizes::{ROW_LABEL_WIDTH, l, m, s},
    text as app_text,
};

/// The control column: a pick list is this wide whatever its entries say, so a
/// board with a long name does not stretch the row.
const CONTROL_WIDTH: f32 = 400.0;
/// A board's parts sit under it, narrower than the board itself.
const PART_LABEL_WIDTH: f32 = 110.0;
const PART_WIDTH: f32 = 160.0;
const PANEL_WIDTH: f32 = 660.0;
const MAX_PANEL_HEIGHT: f32 = 640.0;

/// Everything one launch panel's rows read: the options a family publishes,
/// the user's own word on them, what fills the rest, and where an edit lands.
pub struct PanelData {
    pub descriptors: Vec<LaunchOptionDescriptor>,
    pub overrides: LaunchValues,
    pub facts: Facts,
    pub surface: EditSurface,
}

/// One row per option: its label and a control that reads "Automatic" until the
/// user sets it. Nothing marks an override — a control not reading "Automatic"
/// is the mark.
pub fn panel(data: &PanelData) -> Element<'static, app::Message> {
    if data.descriptors.is_empty() {
        return app_text::detail("This system takes no launch options.")
            .color(MUTED)
            .into();
    }

    let mut rows = column![].spacing(m());
    for descriptor in &data.descriptors {
        rows = rows.push(option_row(descriptor, data));
    }
    rows.into()
}

fn option_row(
    descriptor: &LaunchOptionDescriptor,
    data: &PanelData,
) -> Element<'static, app::Message> {
    let control = match &descriptor.kind {
        LaunchOptionKind::Choice { choices } => choice_control(descriptor.id, choices, data),
        LaunchOptionKind::Toggle => toggle_control(descriptor.id, data),
        LaunchOptionKind::File { label } => file_control(descriptor.id, label, data),
        LaunchOptionKind::Board { boards } => board_control(descriptor.id, boards, data),
    };

    // A board's parts stack under its pick list, so the label holds to the top
    // beside that first line rather than centring on the whole stack.
    let label = container(app_text::label(descriptor.label)).width(ROW_LABEL_WIDTH);
    let (label, alignment) = match &descriptor.kind {
        LaunchOptionKind::Board { .. } => (
            label.padding(iced::Padding::ZERO.top(s())),
            Alignment::Start,
        ),
        _ => (label, Center),
    };

    row![label, control].spacing(m()).align_y(alignment).into()
}

/// One entry of an option's pick list: the automatic entry, which names what
/// the launch would use and where that came from, or a value the core accepts.
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    /// `None` is Automatic — choosing it drops the user's word.
    value: Option<String>,
    label: String,
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// The Automatic entry, naming what fills the option.
fn automatic(
    id: &str,
    data: &PanelData,
    describe: impl Fn(&LaunchValue) -> Option<String>,
) -> Entry {
    let label = data
        .facts
        .get(id)
        .and_then(describe)
        .map(|described| format!("Automatic ({described})"))
        .unwrap_or_else(|| "Automatic".to_string());
    Entry { value: None, label }
}

fn choice_control(
    id: &'static str,
    choices: &[missingno_core::launch::LaunchChoice],
    data: &PanelData,
) -> Element<'static, app::Message> {
    let label_of = |code: &str| {
        choices
            .iter()
            .find(|choice| choice.value == code)
            .map(|choice| choice.label.to_string())
    };

    let mut entries = vec![automatic(id, data, |value| match value {
        LaunchValue::Choice(code) => label_of(code).or_else(|| Some(code.clone())),
        _ => None,
    })];
    entries.extend(choices.iter().map(|choice| Entry {
        value: Some(choice.value.to_string()),
        label: choice.label.to_string(),
    }));

    let selected = data.overrides.choice(id).and_then(|code| {
        entries
            .iter()
            .find(|entry| entry.value.as_deref() == Some(code))
            .cloned()
    });
    let selected = Some(selected.unwrap_or_else(|| entries[0].clone()));

    let surface = data.surface;
    pick_list(entries, selected, move |entry| {
        Message::Set(surface, Edit::Choice(id, entry.value)).into()
    })
    .width(CONTROL_WIDTH)
    .into()
}

/// The board option: a pick list over the boards the core builds, and — once
/// the user names one — a row for each part that board's silicon varies in.
fn board_control(
    id: &'static str,
    boards: &[BoardSpec],
    data: &PanelData,
) -> Element<'static, app::Message> {
    let mut entries = vec![automatic(id, data, |value| match value {
        LaunchValue::Board(board) => Some(describe_board(board, boards)),
        _ => None,
    })];
    entries.extend(boards.iter().map(|spec| Entry {
        value: Some(spec.name.to_string()),
        label: spec.display.to_string(),
    }));

    let chosen = data.overrides.board(id).cloned();
    let selected = chosen
        .as_ref()
        .and_then(|board| {
            entries
                .iter()
                .find(|entry| entry.value.as_deref() == Some(board.board.as_str()))
                .cloned()
        })
        .unwrap_or_else(|| entries[0].clone());

    let surface = data.surface;
    let catalogue = boards.to_vec();
    let stated = match data.facts.get(id) {
        Some(LaunchValue::Board(board)) => Some(board.clone()),
        _ => None,
    };
    let picker = pick_list(entries, Some(selected), move |entry| {
        let picked = entry.value.and_then(|name| {
            catalogue
                .iter()
                .find(|spec| spec.name == name)
                .map(|spec| seeded(spec, stated.as_ref()))
        });
        Message::Set(surface, Edit::Board(id, picked)).into()
    })
    .width(CONTROL_WIDTH);

    let mut control = column![picker].spacing(s());
    if let Some(board) = chosen
        && let Some(spec) = boards.iter().find(|spec| spec.name == board.board)
    {
        for attribute in spec.attributes {
            if let Some(part) = part_row(id, surface, &board, attribute) {
                control = control.push(part);
            }
        }
    }
    control.into()
}

/// A stated board for a reader: what the catalogue calls it, then the parts it
/// carries, in the order that board lists them.
fn describe_board(board: &BoardValue, boards: &[BoardSpec]) -> String {
    let Some(spec) = boards.iter().find(|spec| spec.name == board.board) else {
        return board.board.clone();
    };
    let mut parts = vec![spec.display.to_string()];
    for attribute in spec.attributes {
        match board.attributes.get(attribute.key) {
            Some(AttributeValue::Choice(name)) => parts.push(format!("{} {name}", attribute.label)),
            Some(AttributeValue::Toggle(true)) => parts.push(attribute.label.to_string()),
            _ => {}
        }
    }
    parts.join(", ")
}

/// The board a pick starts from: whatever the media already stated that this
/// board takes, and a starting part wherever it carries one either way.
fn seeded(spec: &BoardSpec, stated: Option<&BoardValue>) -> BoardValue {
    let mut board = BoardValue::new(spec.name);
    for attribute in spec.attributes {
        // A byte count is measured off the silicon rather than picked.
        if matches!(attribute.kind, AttributeKind::Bytes) {
            continue;
        }
        let carried = stated
            .and_then(|stated| stated.attributes.get(attribute.key))
            .filter(|value| fits(value, &attribute.kind));
        let part = match carried {
            Some(carried) => Some(carried.clone()),
            None if attribute.optional => None,
            None => match &attribute.kind {
                AttributeKind::Choice { names } => names
                    .first()
                    .map(|name| AttributeValue::Choice((*name).to_string())),
                _ => Some(AttributeValue::Toggle(false)),
            },
        };
        board = board.with_optional(attribute.key, part);
    }
    board
}

/// Whether a part another board stated is one this one takes: the right kind,
/// and a setting its silicon comes in.
fn fits(value: &AttributeValue, kind: &AttributeKind) -> bool {
    match (value, kind) {
        (AttributeValue::Choice(name), AttributeKind::Choice { names }) => {
            names.contains(&name.as_str())
        }
        (AttributeValue::Toggle(_), AttributeKind::Toggle) => true,
        (AttributeValue::Bytes(_), AttributeKind::Bytes) => true,
        _ => false,
    }
}

/// One part of the board the user named, under it: the setting its silicon
/// comes in, or whether it is populated at all. A byte count is measured rather
/// than picked, so it has no row.
fn part_row(
    id: &'static str,
    surface: EditSurface,
    board: &BoardValue,
    attribute: &'static AttributeSpec,
) -> Option<Element<'static, app::Message>> {
    let control: Element<'static, app::Message> = match &attribute.kind {
        AttributeKind::Choice { names } => {
            // A part the board may carry none of is left off by naming none.
            let mut entries: Vec<Entry> = attribute
                .optional
                .then(|| Entry {
                    value: None,
                    label: "None".to_string(),
                })
                .into_iter()
                .collect();
            entries.extend(names.iter().map(|name| Entry {
                value: Some((*name).to_string()),
                label: (*name).to_string(),
            }));

            let stated = match board.attributes.get(attribute.key) {
                Some(AttributeValue::Choice(name)) => Some(name.as_str()),
                _ => None,
            };
            let selected = entries
                .iter()
                .find(|entry| entry.value.as_deref() == stated)
                .cloned();

            let board = board.clone();
            pick_list(entries, selected, move |entry| {
                let stated = match entry.value {
                    Some(name) => board.clone().with_choice(attribute.key, &name),
                    None => {
                        let mut cleared = board.clone();
                        cleared.attributes.remove(attribute.key);
                        cleared
                    }
                };
                Message::Set(surface, Edit::Board(id, Some(stated))).into()
            })
            .width(PART_WIDTH)
            .into()
        }
        AttributeKind::Toggle => {
            let on = matches!(
                board.attributes.get(attribute.key),
                Some(AttributeValue::Toggle(true))
            );
            let board = board.clone();
            toggler(on)
                .on_toggle(move |on| {
                    let stated = board.clone().with_toggle(attribute.key, on);
                    Message::Set(surface, Edit::Board(id, Some(stated))).into()
                })
                .size(m())
                .into()
        }
        AttributeKind::Bytes => return None,
    };

    Some(
        row![
            container(app_text::detail(attribute.label).color(MUTED)).width(PART_LABEL_WIDTH),
            control,
        ]
        .spacing(s())
        .align_y(Center)
        .into(),
    )
}

/// A toggle is picked the same way a choice is, so that leaving it automatic
/// stays one entry of the same list rather than a third state of a checkbox.
fn toggle_control(id: &'static str, data: &PanelData) -> Element<'static, app::Message> {
    let word = |on: bool| if on { "On" } else { "Off" };

    let mut entries = vec![automatic(id, data, |value| match value {
        LaunchValue::Toggle(on) => Some(word(*on).to_string()),
        _ => None,
    })];
    entries.extend([true, false].map(|on| Entry {
        value: Some(word(on).to_string()),
        label: word(on).to_string(),
    }));

    let selected = data
        .overrides
        .value(id)
        .and_then(|value| match value {
            LaunchValue::Toggle(on) => Some(word(*on)),
            _ => None,
        })
        .and_then(|chosen| {
            entries
                .iter()
                .find(|entry| entry.value.as_deref() == Some(chosen))
                .cloned()
        });
    let selected = Some(selected.unwrap_or_else(|| entries[0].clone()));

    let surface = data.surface;
    pick_list(entries, selected, move |entry| {
        Message::Set(
            surface,
            Edit::Toggle(id, entry.value.map(|word| word == "On")),
        )
        .into()
    })
    .width(CONTROL_WIDTH)
    .into()
}

fn file_control(
    id: &'static str,
    label: &'static str,
    data: &PanelData,
) -> Element<'static, app::Message> {
    let surface = data.surface;
    let chosen = data.overrides.file(id).map(<[u8]>::len);

    let status = match chosen {
        Some(bytes) => format!("Chosen · {bytes} bytes"),
        None => "Automatic".to_string(),
    };

    let mut controls = row![
        buttons::standard(iced::widget::text(format!("Choose {label}…")))
            .on_press(Message::PickFile(surface, id).into()),
        app_text::detail(status).color(MUTED),
    ]
    .spacing(s())
    .align_y(Center);

    if chosen.is_some() {
        controls = controls.push(
            buttons::subtle("Automatic")
                .on_press(Message::Set(surface, Edit::File(id, None)).into()),
        );
    }

    controls.into()
}

/// The launch window: what is about to boot, the options it will boot with, and
/// the one keystroke that starts it.
pub fn window(state: &Window) -> Element<'static, app::Message> {
    let mut heading = column![app_text::heading(state.title.clone())].spacing(4);
    if let Some(platform) = state.platform {
        heading = heading.push(app_text::detail(platform.name()).color(MUTED));
    }

    let mut body = column![heading].spacing(l());

    if !state.claimed {
        body = body.push(
            column![
                app_text::detail("No system claims this file. Choose the one it is for.")
                    .color(MUTED),
                row![
                    container(app_text::label("System")).width(ROW_LABEL_WIDTH),
                    pick_list(platforms_by_name(), state.platform, |platform: Platform| {
                        Message::SelectSystem(platform).into()
                    })
                    .placeholder("Choose a system")
                    .width(CONTROL_WIDTH),
                ]
                .spacing(m())
                .align_y(Center),
            ]
            .spacing(m()),
        );
    }

    if let Some(family) = state.family() {
        body = body.push(horizontal_rule());
        body = body.push(panel(&PanelData {
            descriptors: super::rendered_options(family, &state.rom),
            overrides: state.overrides.clone(),
            facts: state.facts.clone(),
            surface: EditSurface::Window,
        }));
    }

    if let Some(error) = &state.error {
        body = body.push(app_text::detail(error.clone()).color(RED));
    }

    let launch = buttons::primary("Launch");
    let launch = if state.family().is_some() {
        launch.on_press(Message::Launch.into())
    } else {
        launch
    };

    body = body.push(
        row![
            buttons::standard("Cancel").on_press(Message::Close.into()),
            launch,
        ]
        .spacing(s()),
    );

    opaque(
        mouse_area(
            center(
                container(scrollable(container(body).padding(l())).width(PANEL_WIDTH))
                    .max_height(MAX_PANEL_HEIGHT)
                    .style(containers::menu),
            )
            .style(|_| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                ..Default::default()
            }),
        )
        .on_press(Message::Close.into()),
    )
}
