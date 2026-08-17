//! The launch panel and the window that stands it over the screen. Rows are
//! rendered from the family's descriptors alone, so nothing here names an
//! option.

use iced::{
    Alignment::Center,
    Element,
    widget::{center, column, container, mouse_area, opaque, pick_list, row, scrollable},
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
    };

    row![
        container(app_text::label(descriptor.label)).width(ROW_LABEL_WIDTH),
        control,
    ]
    .spacing(m())
    .align_y(Center)
    .into()
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
