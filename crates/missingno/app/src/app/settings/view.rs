use std::collections::HashMap;
use std::path::PathBuf;

use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    widget::{column, container, row, svg, text, toggler},
};
use missingno_core::ports::{ControlKind, PeripheralDescriptor, PeripheralId, Provider};
use missingno_core::system::ControlRole;
use missingno_core::video::DisplayTechnology;
use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};

use crate::app::{
    self, automation, controls,
    settings::{ControlSlot, EMULATOR_ACTIONS, EmulatorAction, Surface, WindDirection},
    system::{Platform, family_of},
    ui::{
        buttons, containers, horizontal_rule,
        icons::{self, Icon},
        palette::{MUTED, YELLOW},
        sizes::{l, m, s},
        text as app_text, vertical_rule,
    },
};

use app_text::TextPart;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    General,
    Display,
    Controls,
    Hardware,
    Developer,
}

/// Which page of the Controls section is showing: the emulator's own actions,
/// or one system's controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlsPage {
    #[default]
    Emulator,
    System(Platform),
}

/// The controller type each system page's Controllers tabs have selected. A
/// system with no entry shows the first controller any of its ports accepts.
pub type ControllerTabs = HashMap<Platform, PeripheralId>;

/// What the Controls section is showing: its page, and that page's controller tab.
#[derive(Debug, Clone, Default)]
pub struct ControlsState {
    pub page: ControlsPage,
    pub controller_tabs: ControllerTabs,
}

/// What a binding row binds: an emulator action, or one control of one system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingTarget {
    Emulator(EmulatorAction),
    System(Platform, ControlSlot),
}

/// What binding we're waiting for input on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListeningFor {
    pub surface: Surface,
    pub target: BindingTarget,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectSection(Section),
    SetInternetEnabled(bool),
    PickRomDirectory,
    AddRomDirectory(PathBuf),
    RemoveRomDirectory(usize),
    SelectPalette(PaletteChoice),
    SetUseSgbColors(bool),
    SetPersistence(bool),
    SetPixelGrid(bool),
    SetScanlines(bool),
    SetHasheousEnabled(bool),
    SetHomebrewHubEnabled(bool),
    SetCartridgeRwEnabled(bool),
    SetAllowExternalClients(bool),
    SetAllowUiAutomation(bool),
    SelectControlsPage(ControlsPage),
    SelectControllerTab(Platform, PeripheralId),
    SetPointerKnob(Platform, bool),
    StartListening(ListeningFor),
    CaptureBinding(String),
    ClearBinding,
    CancelCapture,
    ResetBindings(ControlsPage),
    Back,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Settings(message)
    }
}

pub(in crate::app) fn view<'a>(
    settings: &'a super::Settings,
    section: Section,
    controls: &ControlsState,
    listening_for: Option<ListeningFor>,
    detected_cartridge_devices: &'a [crate::cartridge_rw::DetectedDevice],
) -> Element<'a, app::Message> {
    let sidebar = sidebar_view(section);
    let content = match section {
        Section::Display => display_section(settings),
        Section::General => general_section(settings),
        Section::Controls => controls_section(settings, controls, listening_for),
        Section::Hardware => hardware_section(settings, detected_cartridge_devices),
        Section::Developer => developer_section(settings),
    };

    column![
        row![
            automation::tag(
                automation::ids::SETTINGS_BACK,
                buttons::subtle(icons::m(Icon::Back)).on_press(Message::Back.into())
            ),
            app_text::heading("Settings"),
        ]
        .spacing(s())
        .padding(m())
        .align_y(Center),
        horizontal_rule(),
        row![sidebar, content].height(Fill),
    ]
    .height(Fill)
    .into()
}

/// The automation id-name for a settings section, matching the registry.
fn section_id_name(section: Section) -> &'static str {
    match section {
        Section::General => "general",
        Section::Display => "display",
        Section::Controls => "controls",
        Section::Hardware => "hardware",
        Section::Developer => "developer",
    }
}

fn sidebar_view(current: Section) -> Element<'static, app::Message> {
    let sections = [
        (Section::General, Icon::Sliders, "General"),
        (Section::Display, Icon::Monitor, "Display"),
        (Section::Controls, Icon::Gamepad, "Controls"),
        (Section::Hardware, Icon::CircuitBoard, "Hardware"),
        (Section::Developer, Icon::Debug, "Developer"),
    ];

    let mut col = column![].spacing(s());

    for (section, icon, label) in sections {
        let label_row = row![icons::m(icon), text(label)]
            .spacing(s())
            .align_y(Center);
        let btn = if section == current {
            buttons::selected(label_row).width(Fill)
        } else {
            buttons::subtle(label_row)
                .on_press(Message::SelectSection(section).into())
                .width(Fill)
        };

        col = col.push(automation::tag(
            &automation::ids::section(section_id_name(section)),
            btn,
        ));
    }

    container(col.padding(m()))
        .width(220)
        .height(Fill)
        .style(containers::sidebar)
        .into()
}

// ── Controls ──────────────────────────────────────────────────────────

const ROW_LABEL_WIDTH: f32 = 170.0;
const BINDING_WIDTH: f32 = 140.0;
/// The page rail, wide enough for the longest platform name to wrap into.
const PAGE_RAIL_WIDTH: f32 = 240.0;

/// The pages the Controls section offers: the emulator's own actions first, then
/// one per registered system family, alphabetically.
pub(in crate::app) fn controls_pages() -> Vec<ControlsPage> {
    std::iter::once(ControlsPage::Emulator)
        .chain(
            app::system::platforms_by_name()
                .into_iter()
                .map(ControlsPage::System),
        )
        .collect()
}

fn page_label(page: ControlsPage) -> &'static str {
    match page {
        ControlsPage::Emulator => "Emulator",
        ControlsPage::System(platform) => platform.name(),
    }
}

/// The page's name in an automation id: its label lowercased, spaces underscored.
fn page_id_name(page: ControlsPage) -> String {
    page_label(page).to_lowercase().replace(' ', "_")
}

/// One page's contents, read off the seam descriptors: what the view lays out and
/// what automation enumerates, so the two cannot drift apart.
struct Page {
    groups: Vec<Group>,
}

/// A block of one page: the emulator's actions, the console's own controls, or
/// the controllers its ports take, showing whichever type the tabs select.
struct Group {
    heading: Option<String>,
    tabs: Vec<Tab>,
    rows: Vec<Row>,
}

/// One controller type this system's ports accept, as its tab.
struct Tab {
    platform: Platform,
    peripheral: PeripheralId,
    label: String,
    selected: bool,
}

struct Row {
    label: String,
    /// The control's name qualified by the controller carrying it, for the
    /// accessible names automation reads.
    qualified: String,
    kind: RowKind,
}

enum RowKind {
    Bindable(BindingTarget),
    /// A setting rather than a binding: whether the pointer over the screen
    /// turns this system's knobs.
    Pointer {
        platform: Platform,
        on: bool,
    },
}

/// What the pointer row does, since its name alone does not say.
const POINTER_CAPTION: &str = "Pointer position drives the knob.";

fn page_contents(page: ControlsPage, controller_tabs: &ControllerTabs, pointer_knob: bool) -> Page {
    match page {
        ControlsPage::Emulator => Page {
            groups: vec![Group {
                heading: None,
                tabs: Vec::new(),
                rows: EMULATOR_ACTIONS
                    .iter()
                    .map(|&action| Row {
                        label: action.to_string(),
                        qualified: action.to_string(),
                        kind: RowKind::Bindable(BindingTarget::Emulator(action)),
                    })
                    .collect(),
            }],
        },
        ControlsPage::System(platform) => system_page(platform, controller_tabs, pointer_knob),
    }
}

/// A system's controls in the order they sit on the hardware: what is on the
/// console itself, then the controllers its ports take. Bindings key on the
/// controller type, so a type its ports share is shown once.
fn system_page(platform: Platform, controller_tabs: &ControllerTabs, pointer_knob: bool) -> Page {
    let Some(family) = family_of(platform) else {
        return Page { groups: Vec::new() };
    };
    let controls = &family.controls;
    let mut groups = Vec::new();

    // A console's own knob would need a slot vocabulary of its own; no family
    // declares one, so only its buttons bind here.
    let mut on_unit: Vec<Row> = controls
        .integrated
        .iter()
        .filter(|control| control.kind == ControlKind::Button)
        .map(|control| Row {
            label: control.label.to_string(),
            qualified: control.label.to_string(),
            kind: RowKind::Bindable(BindingTarget::System(
                platform,
                ControlSlot::Integrated(control.role),
            )),
        })
        .collect();
    // Latching switches bind like anything else: a bound press flips them.
    on_unit.extend(controls.panel.iter().map(|control| Row {
        label: control.label.to_string(),
        qualified: control.label.to_string(),
        kind: RowKind::Bindable(BindingTarget::System(
            platform,
            ControlSlot::Panel(control.role),
        )),
    }));
    if !on_unit.is_empty() {
        groups.push(Group {
            heading: None,
            tabs: Vec::new(),
            rows: on_unit,
        });
    }

    let controllers = controller_types(platform);
    if let Some(shown) = controllers
        .iter()
        .find(|peripheral| controller_tabs.get(&platform) == Some(&peripheral.id))
        .or_else(|| controllers.first())
    {
        groups.push(Group {
            heading: Some("Controllers".to_string()),
            tabs: controllers
                .iter()
                .map(|peripheral| Tab {
                    platform,
                    peripheral: peripheral.id,
                    label: peripheral.label.to_string(),
                    selected: peripheral.id == shown.id,
                })
                .collect(),
            rows: shown
                .controls
                .iter()
                .flat_map(|control| {
                    let qualify = |label: &str| format!("{} {label}", shown.label);
                    match control.kind {
                        ControlKind::Button => vec![Row {
                            label: control.label.to_string(),
                            qualified: qualify(control.label),
                            kind: RowKind::Bindable(BindingTarget::System(
                                platform,
                                ControlSlot::Peripheral {
                                    peripheral: shown.id,
                                    role: control.role,
                                },
                            )),
                        }],
                        ControlKind::Axis => {
                            knob_rows(platform, shown.id, control.role, &qualify, pointer_knob)
                        }
                    }
                })
                .collect(),
        });
    }

    Page { groups }
}

/// What turns a knob: the pointer, as a setting, then each way round as its own
/// binding. The knob itself has no row — nothing binds a position.
fn knob_rows(
    platform: Platform,
    peripheral: PeripheralId,
    role: ControlRole,
    qualify: &dyn Fn(&str) -> String,
    pointer_knob: bool,
) -> Vec<Row> {
    let mut rows = vec![Row {
        label: "Mouse".to_string(),
        qualified: qualify("Mouse"),
        kind: RowKind::Pointer {
            platform,
            on: pointer_knob,
        },
    }];
    rows.extend(WindDirection::BOTH.map(|direction| Row {
        label: direction.label().to_string(),
        qualified: qualify(direction.label()),
        kind: RowKind::Bindable(BindingTarget::System(
            platform,
            ControlSlot::Wind {
                peripheral,
                role,
                direction,
            },
        )),
    }));
    rows
}

/// Every controller type this system's ports take, in port order and deduped —
/// a type both jacks accept binds once. Peripherals the host supplies, and ones
/// carrying no controls, are not controllers.
pub(in crate::app) fn controller_types(platform: Platform) -> Vec<&'static PeripheralDescriptor> {
    let Some(family) = family_of(platform) else {
        return Vec::new();
    };
    let mut types: Vec<&'static PeripheralDescriptor> = Vec::new();
    for port in family.controls.ports {
        for peripheral in port.accepts {
            if peripheral.provider == Provider::Console
                && !peripheral.controls.is_empty()
                && !types.iter().any(|known| known.id == peripheral.id)
            {
                types.push(peripheral);
            }
        }
    }
    types
}

fn controls_section<'a>(
    settings: &'a super::Settings,
    controls: &ControlsState,
    listening_for: Option<ListeningFor>,
) -> Element<'a, app::Message> {
    let page = controls.page;
    let mut body = column![
        row![
            iced::widget::Space::new().width(ROW_LABEL_WIDTH),
            app_text::label("Keyboard").width(BINDING_WIDTH),
            app_text::label("Controller").width(BINDING_WIDTH),
        ]
        .spacing(s())
    ]
    .spacing(m());

    let pointer_knob = page_pointer_knob(page, settings);
    for group in page_contents(page, &controls.controller_tabs, pointer_knob).groups {
        if let Some(heading) = group.heading {
            body = body.push(horizontal_rule());
            body = body.push(app_text::label(heading));
        }
        if !group.tabs.is_empty() {
            body = body.push(tab_row(page, group.tabs));
        }
        let mut rows = column![].spacing(s());
        for entry in group.rows {
            rows = rows.push(control_row(entry, page, settings, listening_for));
        }
        body = body.push(rows);
    }

    let body = body
        .push(horizontal_rule())
        .push(automation::tag(
            &automation::ids::controls_reset(&page_id_name(page)),
            buttons::standard(text(format!("Reset {} controls", page_label(page))))
                .on_press(Message::ResetBindings(page).into()),
        ))
        .max_width(620);

    row![
        page_rail(page),
        vertical_rule(),
        iced::widget::scrollable(container(body).padding(l()).width(Fill)).height(Fill),
    ]
    .height(Fill)
    .into()
}

/// Whether the pointer turns the knobs of the system this page shows.
pub(in crate::app) fn page_pointer_knob(page: ControlsPage, settings: &super::Settings) -> bool {
    match page {
        ControlsPage::Emulator => true,
        ControlsPage::System(platform) => settings.controls.pointer_knob(platform),
    }
}

/// The page selector: a rail of entries beside the page they select, so a family
/// joining `FAMILIES` needs no room made for it.
fn page_rail(current: ControlsPage) -> Element<'static, app::Message> {
    let mut col = column![].spacing(s());

    for page in controls_pages() {
        // Raw buttons: a long platform name wraps rather than being clipped to a
        // single line's height.
        let label = text(page_label(page));
        let entry = if page == current {
            buttons::selected_raw(label).width(Fill)
        } else {
            buttons::subtle_raw(label)
                .on_press(Message::SelectControlsPage(page).into())
                .width(Fill)
        };
        col = col.push(automation::tag(
            &automation::ids::controls_page(&page_id_name(page)),
            entry,
        ));
    }

    container(col.padding(m()))
        .width(PAGE_RAIL_WIDTH)
        .height(Fill)
        .into()
}

/// The Controllers block's tabs: which controller type's controls it shows.
fn tab_row(page: ControlsPage, tabs: Vec<Tab>) -> Element<'static, app::Message> {
    let mut tabs_row = row![].spacing(s());

    for tab in tabs {
        let id = automation::ids::controls_tab(&tab_id_name(page, tab.peripheral));
        let label = text(tab.label);
        let entry = if tab.selected {
            buttons::selected(label)
        } else {
            buttons::subtle(label)
                .on_press(Message::SelectControllerTab(tab.platform, tab.peripheral).into())
        };
        tabs_row = tabs_row.push(automation::tag(&id, entry));
    }

    tabs_row.into()
}

/// One row of a page: a control's two binding buttons, or a switch for what a
/// binding cannot say.
fn control_row(
    entry: Row,
    page: ControlsPage,
    settings: &super::Settings,
    listening_for: Option<ListeningFor>,
) -> Element<'static, app::Message> {
    match entry.kind {
        RowKind::Bindable(target) => binding_row(&entry, target, page, settings, listening_for),
        RowKind::Pointer { platform, on } => column![
            automation::tag(
                &automation::ids::controls_option(&pointer_id_name(page)),
                toggler(on)
                    .label(entry.label.clone())
                    .on_toggle(move |drives| Message::SetPointerKnob(platform, drives).into())
                    .size(m()),
            ),
            text(POINTER_CAPTION).color(MUTED),
        ]
        .spacing(s())
        .into(),
    }
}

/// The key or button this target answers to on one surface.
fn binding_of(
    settings: &super::Settings,
    surface: Surface,
    target: BindingTarget,
) -> Option<String> {
    match target {
        BindingTarget::Emulator(action) => settings.controls.emulator_binding(surface, action),
        BindingTarget::System(platform, slot) => {
            settings.controls.system_binding(platform, surface, slot)
        }
    }
}

fn binding_row(
    entry: &Row,
    target: BindingTarget,
    page: ControlsPage,
    settings: &super::Settings,
    listening_for: Option<ListeningFor>,
) -> Element<'static, app::Message> {
    let button_for = |surface: Surface| {
        let listening = listening_for == Some(ListeningFor { surface, target });
        let button = if listening {
            buttons::primary(text("Press key…").color(iced::Color::WHITE)).width(BINDING_WIDTH)
        } else {
            let display = binding_of(settings, surface, target)
                .map(|bound| match surface {
                    Surface::Keyboard => controls::display_key_name(&bound).to_string(),
                    Surface::Gamepad => controls::display_gamepad_name(&bound).to_string(),
                })
                .unwrap_or_else(|| "—".to_string());
            buttons::standard(text(display))
                .on_press(Message::StartListening(ListeningFor { surface, target }).into())
                .width(BINDING_WIDTH)
        };
        automation::tag(
            &automation::ids::controls_binding(&binding_id_name(page, target, surface)),
            button,
        )
    };

    row![
        text(entry.label.clone()).width(ROW_LABEL_WIDTH),
        button_for(Surface::Keyboard),
        button_for(Surface::Gamepad),
    ]
    .spacing(s())
    .align_y(Center)
    .into()
}

// ── Controls automation surface ───────────────────────────────────────

fn tab_id_name(page: ControlsPage, peripheral: PeripheralId) -> String {
    format!("{}.peripheral{}", page_id_name(page), peripheral.0)
}

/// The pointer switch's name in an automation id: one per system, wherever a
/// knob puts it on screen.
fn pointer_id_name(page: ControlsPage) -> String {
    format!("{}.pointer_knob", page_id_name(page))
}

/// A binding button's name in an automation id: the page, the control the way the
/// seam spells it, and the surface being bound.
fn binding_id_name(page: ControlsPage, target: BindingTarget, surface: Surface) -> String {
    let control = match target {
        BindingTarget::Emulator(action) => action.to_string().to_lowercase().replace(' ', "_"),
        BindingTarget::System(_, ControlSlot::Integrated(role)) => {
            format!("integrated.{}", role.name())
        }
        BindingTarget::System(_, ControlSlot::Panel(role)) => format!("panel.{}", role.name()),
        BindingTarget::System(_, ControlSlot::Peripheral { peripheral, role }) => {
            format!("peripheral{}.{}", peripheral.0, role.name())
        }
        BindingTarget::System(
            _,
            ControlSlot::Wind {
                peripheral,
                role,
                direction,
            },
        ) => format!(
            "peripheral{}.{}.{}",
            peripheral.0,
            role.name(),
            direction.id_name()
        ),
    };
    let surface = match surface {
        Surface::Keyboard => "keyboard",
        Surface::Gamepad => "gamepad",
    };
    format!("{}.{control}.{surface}", page_id_name(page))
}

/// One pressable element of the Controls section: its id, its accessible name,
/// and the message pressing it sends. Informational rows are not listed — there
/// is nothing to press on them.
pub(in crate::app) struct ControlsElement {
    pub id: String,
    pub label: String,
    /// Whether the element is a switch rather than a button to press.
    pub toggle: bool,
    pub message: Message,
}

/// Every pressable element the Controls section shows, in reading order: the page
/// selector, the Controllers block's tabs, both binding buttons of every bindable
/// row, and the page's reset.
pub(in crate::app) fn controls_elements(
    controls: &ControlsState,
    pointer_knob: bool,
) -> Vec<ControlsElement> {
    let page = controls.page;
    let current = |shown: bool| if shown { " (current)" } else { "" };
    let mut elements: Vec<ControlsElement> = controls_pages()
        .into_iter()
        .map(|entry| ControlsElement {
            id: automation::ids::controls_page(&page_id_name(entry)),
            label: format!(
                "Show {} controls{}",
                page_label(entry),
                current(entry == page)
            ),
            toggle: false,
            message: Message::SelectControlsPage(entry),
        })
        .collect();

    for group in page_contents(page, &controls.controller_tabs, pointer_knob).groups {
        for tab in group.tabs {
            elements.push(ControlsElement {
                id: automation::ids::controls_tab(&tab_id_name(page, tab.peripheral)),
                label: format!("Show {} controls{}", tab.label, current(tab.selected)),
                toggle: false,
                message: Message::SelectControllerTab(tab.platform, tab.peripheral),
            });
        }
        for entry in group.rows {
            match entry.kind {
                RowKind::Bindable(target) => {
                    for (surface, name) in [
                        (Surface::Keyboard, "keyboard"),
                        (Surface::Gamepad, "controller"),
                    ] {
                        elements.push(ControlsElement {
                            id: automation::ids::controls_binding(&binding_id_name(
                                page, target, surface,
                            )),
                            label: format!("Bind {} on the {name}", entry.qualified),
                            toggle: false,
                            message: Message::StartListening(ListeningFor { surface, target }),
                        });
                    }
                }
                RowKind::Pointer { platform, on } => elements.push(ControlsElement {
                    id: automation::ids::controls_option(&pointer_id_name(page)),
                    label: "Turn the knob with the pointer".to_string(),
                    toggle: true,
                    message: Message::SetPointerKnob(platform, !on),
                }),
            }
        }
    }

    elements.push(ControlsElement {
        id: automation::ids::controls_reset(&page_id_name(page)),
        label: format!("Reset {} controls to defaults", page_label(page)),
        toggle: false,
        message: Message::ResetBindings(page),
    });
    elements
}

// ── Display ───────────────────────────────────────────────────────────

/// An effect the renderer applies over the console's frames. Persistence is
/// every screen's — the renderer scales its strength to the technology — while
/// each overlay belongs to one screen type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Persistence,
    Scanlines,
    PixelGrid,
}

impl Effect {
    pub const ALL: [Effect; 3] = [Effect::Persistence, Effect::Scanlines, Effect::PixelGrid];

    fn label(self) -> &'static str {
        match self {
            Effect::Persistence => "Screen persistence",
            Effect::Scanlines => "Scanlines",
            Effect::PixelGrid => "Pixel grid",
        }
    }

    /// What the effect does and which screens it reaches.
    fn caption(self) -> &'static str {
        match self {
            Effect::Persistence => {
                "Blends consecutive frames the way the screen's pixels lag; strength is adjusted \
                 to the system's display."
            }
            Effect::Scanlines => "Darkens between the picture's lines. TV-output systems.",
            Effect::PixelGrid => "Separates the panel's pixels with a faint grid. LCD handhelds.",
        }
    }

    fn id_name(self) -> &'static str {
        match self {
            Effect::Persistence => "persistence",
            Effect::Scanlines => "scanlines",
            Effect::PixelGrid => "pixel_grid",
        }
    }

    fn set(self, enabled: bool) -> Message {
        match self {
            Effect::Persistence => Message::SetPersistence(enabled),
            Effect::Scanlines => Message::SetScanlines(enabled),
            Effect::PixelGrid => Message::SetPixelGrid(enabled),
        }
    }

    /// Whether a screen of this technology shows the effect.
    fn shows_on(self, technology: DisplayTechnology) -> bool {
        match self {
            Effect::Persistence => true,
            Effect::Scanlines => matches!(technology, DisplayTechnology::Crt { .. }),
            Effect::PixelGrid => matches!(technology, DisplayTechnology::Lcd { .. }),
        }
    }
}

/// Where each effect switch stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Effects {
    pub persistence: bool,
    pub scanlines: bool,
    pub pixel_grid: bool,
}

impl Effects {
    fn get(self, effect: Effect) -> bool {
        match effect {
            Effect::Persistence => self.persistence,
            Effect::Scanlines => self.scanlines,
            Effect::PixelGrid => self.pixel_grid,
        }
    }
}

/// The display options a surface offers. Both surfaces set the same settings:
/// the settings screen states no technology and lists every effect with the
/// screens it reaches, while the play panel states the running console's and
/// lists only what that screen shows.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisplayOptions {
    pub effects: Effects,
    pub technology: Option<DisplayTechnology>,
    /// `Some(position)` where Super Game Boy colours are offered.
    pub sgb_colors: Option<bool>,
    /// `Some(choice)` where a monochrome palette is selectable.
    pub palette: Option<PaletteChoice>,
}

impl DisplayOptions {
    /// The effect switches this surface shows.
    pub fn effect_rows(&self) -> Vec<DisplayRow> {
        Effect::ALL
            .into_iter()
            .filter(|effect| {
                self.technology
                    .is_none_or(|technology| effect.shows_on(technology))
            })
            .map(|effect| DisplayRow::Effect {
                effect,
                on: self.effects.get(effect),
            })
            .collect()
    }

    /// The Game Boy colour rows this surface shows, empty where the console has
    /// no colour choice to make.
    pub fn game_boy_rows(&self) -> Vec<DisplayRow> {
        let sgb = self.sgb_colors.map(DisplayRow::SgbColors);
        let palettes = self.palette.into_iter().flat_map(|selected| {
            PaletteChoice::ALL
                .iter()
                .map(move |&choice| DisplayRow::Palette {
                    choice,
                    selected: choice == selected,
                })
        });
        sgb.into_iter().chain(palettes).collect()
    }

    /// Every row, in reading order — what automation enumerates.
    pub fn rows(&self) -> Vec<DisplayRow> {
        let mut rows = self.effect_rows();
        rows.extend(self.game_boy_rows());
        rows
    }
}

/// One row of the display options: a switch, or a palette to pick.
#[derive(Debug, Clone, Copy)]
pub enum DisplayRow {
    Effect {
        effect: Effect,
        on: bool,
    },
    SgbColors(bool),
    Palette {
        choice: PaletteChoice,
        selected: bool,
    },
}

impl DisplayRow {
    /// The row's name in an automation id, qualified by its group.
    pub fn id_name(&self) -> String {
        match self {
            DisplayRow::Effect { effect, .. } => format!("effects.{}", effect.id_name()),
            DisplayRow::SgbColors(_) => "game_boy.sgb_colors".to_string(),
            DisplayRow::Palette { choice, .. } => {
                format!("game_boy.palette.{}", choice.to_string().to_lowercase())
            }
        }
    }

    /// The row's accessible name.
    pub fn label(&self) -> String {
        match self {
            DisplayRow::Effect { effect, .. } => effect.label().to_string(),
            DisplayRow::SgbColors(_) => "Super Game Boy colours".to_string(),
            DisplayRow::Palette { choice, selected } => {
                let current = if *selected { " (current)" } else { "" };
                format!("Use the {choice} palette{current}")
            }
        }
    }

    /// What the row applies to, where that is not obvious from its name.
    pub fn caption(&self) -> Option<&'static str> {
        match self {
            DisplayRow::Effect { effect, .. } => Some(effect.caption()),
            DisplayRow::SgbColors(_) => {
                Some("Colours games the way a Super Game Boy would. Off uses the palette below.")
            }
            DisplayRow::Palette { .. } => None,
        }
    }

    /// Where a switch row stands; `None` on a row that is pressed, not switched.
    pub fn switched_on(&self) -> Option<bool> {
        match self {
            DisplayRow::Effect { on, .. } => Some(*on),
            DisplayRow::SgbColors(on) => Some(*on),
            DisplayRow::Palette { .. } => None,
        }
    }

    /// The message setting this row to `enabled`; a palette tile has one
    /// position, so it selects itself either way.
    pub fn set(&self, enabled: bool) -> Message {
        match self {
            DisplayRow::Effect { effect, .. } => effect.set(enabled),
            DisplayRow::SgbColors(_) => Message::SetUseSgbColors(enabled),
            DisplayRow::Palette { choice, .. } => Message::SelectPalette(*choice),
        }
    }

    /// The message activating the row: a switch flips, a tile selects.
    pub fn activate(&self) -> Message {
        match self.switched_on() {
            Some(on) => self.set(!on),
            None => self.set(true),
        }
    }
}

/// One control of a display surface: its id, its accessible name, whether it is
/// a switch, and the message activating it.
pub(in crate::app) struct DisplayElement {
    pub id: String,
    pub label: String,
    pub toggle: bool,
    pub message: Message,
}

/// Every control a display surface shows, in reading order, with its ids made
/// the way that surface names them — the settings section and the play panel
/// offer the same rows under their own prefixes.
pub(in crate::app) fn display_elements(
    options: &DisplayOptions,
    id: fn(&str) -> String,
) -> Vec<DisplayElement> {
    options
        .rows()
        .into_iter()
        .map(|entry| DisplayElement {
            id: id(&entry.id_name()),
            label: entry.label(),
            toggle: entry.switched_on().is_some(),
            message: entry.activate(),
        })
        .collect()
}

/// The Display section: every effect with the screens it reaches, then the
/// colour options of the one console family that has any. Which effects a
/// running console actually shows is the play panel's filter, not this screen's.
fn display_section(settings: &super::Settings) -> Element<'_, app::Message> {
    let options = settings.display_options();

    let mut effects = column![app_text::label("Effects")].spacing(m());
    for entry in options.effect_rows() {
        effects = effects.push(display_switch(entry));
    }

    let mut content = column![effects].spacing(m());

    let game_boy = options.game_boy_rows();
    if !game_boy.is_empty() {
        let mut group = column![app_text::label("Game Boy")].spacing(m());
        let mut palettes = row![].spacing(m());
        for entry in game_boy {
            match entry {
                DisplayRow::Palette { choice, selected } => {
                    palettes = palettes.push(palette_tile(
                        &entry.id_name(),
                        choice,
                        selected,
                        entry.activate(),
                    ))
                }
                _ => group = group.push(display_switch(entry)),
            }
        }
        content = content.push(horizontal_rule());
        content = content.push(group.push(palettes));
    }

    let content = content.max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

/// One switch of the Display section, over the caption saying which screens it
/// reaches.
fn display_switch(entry: DisplayRow) -> Element<'static, app::Message> {
    let mut col = column![automation::tag(
        &automation::ids::settings_display_row(&entry.id_name()),
        toggler(entry.switched_on().unwrap_or(false))
            .label(entry.label())
            .on_toggle(move |enabled| entry.set(enabled).into())
            .size(m()),
    )]
    .spacing(s());
    if let Some(caption) = entry.caption() {
        col = col.push(text(caption).color(MUTED));
    }
    col.into()
}

/// One palette as a tile: its four shades over its name.
fn palette_tile(
    id_name: &str,
    choice: PaletteChoice,
    selected: bool,
    select: Message,
) -> Element<'static, app::Message> {
    let palette = choice.palette();
    let swatches = row![
        color_swatch(palette.color(PaletteIndex(0))),
        color_swatch(palette.color(PaletteIndex(1))),
        color_swatch(palette.color(PaletteIndex(2))),
        color_swatch(palette.color(PaletteIndex(3))),
    ]
    .spacing(0);
    let tile_content = column![swatches, text(format!("{choice}"))]
        .spacing(s())
        .align_x(Center);

    let tile = if selected {
        buttons::selected_raw(tile_content)
    } else {
        buttons::subtle_raw(tile_content).on_press(select.into())
    };
    automation::tag(&automation::ids::settings_display_row(id_name), tile)
}

fn color_swatch(color: rgb::RGB8) -> Element<'static, app::Message> {
    let c = iced::Color::from_rgb8(color.r, color.g, color.b);
    container(iced::widget::Space::new().width(40).height(40))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(c.into()),
            ..Default::default()
        })
        .into()
}

fn general_section(settings: &super::Settings) -> Element<'_, app::Message> {
    let version = env!("CARGO_PKG_VERSION").trim_end_matches(".0");

    let about = row![
        icons::xl(Icon::GameBoy)
            .width(64)
            .height(64)
            .style(|_, _| svg::Style { color: None }),
        column![
            app_text::heading("Missingno"),
            app_text::link_text(
                [
                    TextPart::plain(format!("Version {version} · ")),
                    TextPart::link("Website", "https://andyofniall.net/"),
                    TextPart::plain(" · "),
                    TextPart::link("GitHub", "https://github.com/ajoneil/missingno"),
                ],
                MUTED
            ),
        ]
        .spacing(s()),
    ]
    .spacing(m())
    .align_y(Center);

    let mut network = column![
        toggler(settings.internet_enabled)
            .label("Allow internet access")
            .on_toggle(|enabled| Message::SetInternetEnabled(enabled).into())
            .size(m()),
    ]
    .spacing(m());

    if settings.internet_enabled {
        network = network.push(
            column![
                toggler(settings.hasheous_enabled)
                    .label("Game metadata")
                    .on_toggle(|enabled| Message::SetHasheousEnabled(enabled).into())
                    .size(m()),
                app_text::link_text(
                    [
                        TextPart::plain("Provided by "),
                        TextPart::link("Hasheous", "https://hasheous.org"),
                    ],
                    MUTED
                ),
            ]
            .spacing(s()),
        );

        network = network.push(
            column![
                toggler(settings.homebrew_hub_enabled)
                    .label("Homebrew catalogue")
                    .on_toggle(|enabled| Message::SetHomebrewHubEnabled(enabled).into())
                    .size(m()),
                app_text::link_text(
                    [
                        TextPart::plain("Browse free games from "),
                        TextPart::link("Homebrew Hub", "https://hh.gbdev.io"),
                    ],
                    MUTED
                ),
            ]
            .spacing(s()),
        );
    }

    let mut directories = column![].spacing(s());

    for (i, dir) in settings.rom_directories.iter().enumerate() {
        let path_str = dir.to_string_lossy().to_string();
        let path_view: Element<'_, app::Message> = if dir.exists() {
            text(path_str).into()
        } else {
            column![
                row![
                    icons::m_colored(Icon::Warning, YELLOW),
                    text(path_str).color(YELLOW),
                ]
                .spacing(s())
                .align_y(Center),
                app_text::detail("Folder not found").color(MUTED),
            ]
            .spacing(2)
            .into()
        };

        directories = directories.push(
            row![
                container(path_view).width(Fill),
                buttons::danger(icons::m(Icon::Trash))
                    .on_press(Message::RemoveRomDirectory(i).into()),
            ]
            .spacing(s())
            .align_y(Center),
        );
    }

    directories = directories
        .push(buttons::standard("Add folder...").on_press(Message::PickRomDirectory.into()));

    // Flatpak hands out folder access per session unless it is made permanent,
    // so a library folder can go missing after a reboot.
    if cfg!(target_os = "linux") {
        directories = directories.push(app_text::link_text(
            [
                TextPart::plain("If your library folders stop being found after a reboot, use "),
                TextPart::link(
                    "Flatseal",
                    "https://flathub.org/apps/com.github.tchx84.Flatseal",
                ),
                TextPart::plain(" to give Missingno permanent access to them."),
            ],
            MUTED,
        ));
    }

    let content = column![
        about,
        horizontal_rule(),
        app_text::label("Network"),
        network,
        horizontal_rule(),
        app_text::label("Library Folders"),
        directories,
    ]
    .spacing(m())
    .max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

/// The surfaces that let something outside this process reach the running game
/// or the window. Both are off unless the user opts in.
fn developer_section(settings: &super::Settings) -> Element<'_, app::Message> {
    let content = column![
        app_text::label("External Clients"),
        column![
            automation::tag(
                automation::ids::SETTINGS_EXTERNAL_CLIENTS,
                toggler(settings.allow_external_clients)
                    .label("Allow external debugger clients")
                    .on_toggle(|enabled| Message::SetAllowExternalClients(enabled).into())
                    .size(m())
            ),
            text(
                "Lets a debugger or agent signed in as you attach to the running game and drive \
                 it — whatever it does shows in this window. While this is off, nothing outside \
                 Missingno can reach a game."
            )
            .color(MUTED),
        ]
        .spacing(s()),
        column![
            automation::tag(
                automation::ids::SETTINGS_UI_AUTOMATION,
                toggler(settings.allow_ui_automation)
                    .label("Allow UI automation")
                    .on_toggle(|enabled| Message::SetAllowUiAutomation(enabled).into())
                    .size(m())
            ),
            text(
                "Lets a tool signed in as you enumerate this window's controls and press, type, \
                 and scroll them — whatever it does shows here. While this is off, nothing \
                 outside Missingno can drive the window."
            )
            .color(MUTED),
        ]
        .spacing(s()),
    ]
    .spacing(m())
    .max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}

fn hardware_section<'a>(
    settings: &'a super::Settings,
    detected_devices: &'a [crate::cartridge_rw::DetectedDevice],
) -> Element<'a, app::Message> {
    let mut content = column![
        app_text::label("Cartridge Reader/Writer"),
        toggler(settings.cartridge_rw_enabled)
            .label("Enable cartridge reader/writer support")
            .on_toggle(|enabled| Message::SetCartridgeRwEnabled(enabled).into())
            .size(m()),
        app_text::link_text(
            [
                TextPart::plain(
                    "Read and write ROMs and save data from physical Game Boy cartridges using a "
                ),
                TextPart::link("GBxCart RW", "https://www.gbxcart.com/"),
                TextPart::plain(" device."),
            ],
            MUTED
        ),
        app_text::link_text(
            [
                TextPart::plain("For advanced features and broader hardware support, see "),
                TextPart::link("FlashGBX", "https://github.com/lesserkuma/FlashGBX"),
                TextPart::plain("."),
            ],
            MUTED
        ),
    ]
    .spacing(m());

    if settings.cartridge_rw_enabled {
        content = content.push(horizontal_rule());
        content = content.push(app_text::label("Detected Devices"));

        if detected_devices.is_empty() {
            content = content.push(
                text("No devices found. Devices will appear here automatically when connected.")
                    .color(MUTED),
            );
            if cfg!(target_os = "linux") {
                content = content.push(app_text::link_text(
                    [
                        TextPart::plain("You may need to install "),
                        TextPart::link(
                            "udev rules",
                            "https://github.com/ajoneil/missingno#cartridge-readerwriter",
                        ),
                        TextPart::plain(" for the device to be accessible."),
                    ],
                    MUTED,
                ));
            }
        } else {
            for device in detected_devices {
                content = content.push(
                    row![
                        icons::m(Icon::CircuitBoard),
                        column![
                            text(device.display_name()),
                            text(format!(
                                "{} (PCB v{}, FW v{})",
                                device.port_name, device.pcb_version, device.firmware_version
                            ))
                            .color(MUTED),
                        ]
                        .spacing(2),
                    ]
                    .spacing(s())
                    .align_y(Center),
                );
            }
        }
    }

    let content = content.max_width(600);

    iced::widget::scrollable(container(content).padding(l()).width(Fill))
        .height(Fill)
        .into()
}
