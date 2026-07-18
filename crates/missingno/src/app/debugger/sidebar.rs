use std::collections::HashSet;

use iced::{
    Background, Border, Color, Element,
    Length::{self, Fill},
    alignment::Vertical,
    widget::{Space, button, column, container, rule, text, tooltip},
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{self},
    emu_thread::RunningStatus,
    screen::iced_color,
    ui::{
        fonts,
        icons::{self, Icon},
        palette,
        sizes::{s, xs},
    },
};
use missingno_core::inspect;
use missingno_gb::ppu::types::palette::{Palette, PaletteIndex, PaletteMap};

/// Monospace text size for register labels and values.
const REG: f32 = 14.0;
/// Small label size for annotations and the packed video rows.
const LABEL: f32 = 11.0;
/// Detail text size for collapsed summaries and mode accents.
const DETAIL: f32 = 11.0;
/// Column-header text in a bit table — smaller so the wide interrupt table's
/// named columns fit the fixed sidebar width.
const HEADER: f32 = 10.0;

const SIDEBAR_WIDTH: f32 = 260.0;

/// Fixed width for one 8-bit register display ("b 04"), so columns align.
const REG8_WIDTH: f32 = 48.0;
/// Fixed width for a swatch row's label, so swatches line up.
const SWATCH_LABEL_WIDTH: f32 = 40.0;
/// A bit table cell's height, so its header, rows, and pips align across
/// columns.
const CELL_HEIGHT: f32 = 16.0;

/// Content width available to a packed row block, inside the section body's
/// padding. Adjacent short rows coalesce onto one line up to this budget.
const ROW_BUDGET: f32 = 236.0;

/// The number-line width for a period sweep, and its bar height and marker
/// width — sized to sit on one sidebar row beside a label and a value.
const SWEEP_BAR_WIDTH: f32 = 96.0;
const SWEEP_BAR_HEIGHT: f32 = 6.0;
const SWEEP_MARKER_WIDTH: f32 = 2.0;

/// The sidebar over a core's [`inspect::Section`] schema: a stack of
/// collapsible sections, each rendering its typed blocks. Every family renders
/// through the same path — the Game Boy has no bespoke sidebar.
pub struct Sidebar {
    /// Sections the user has collapsed, keyed by section name.
    collapsed: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleSection(String),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Debugger(debugger::Message::Sidebar(message))
    }
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            collapsed: HashSet::new(),
        }
    }

    fn is_collapsed(&self, name: &str) -> bool {
        self.collapsed.contains(name)
    }

    pub fn update(&mut self, message: &Message) {
        match message {
            Message::ToggleSection(name) => {
                if !self.collapsed.remove(name) {
                    self.collapsed.insert(name.clone());
                }
            }
        }
    }

    /// The schema sidebar, fed from the live console while paused or the
    /// per-vblank snapshot while the core runs. `colors` resolves the DMG
    /// shade swatches through the user palette; families that emit none may
    /// pass `None`.
    pub fn view(
        &self,
        sections: Vec<inspect::Section>,
        colors: Option<&ConsoleColors>,
    ) -> Element<'static, app::Message> {
        let mut stack = column![]
            .width(Length::Fixed(SIDEBAR_WIDTH))
            .height(Fill)
            .spacing(s());
        for section in sections {
            stack = stack.push(self.render_section(section, colors));
        }
        stack.into()
    }

    fn render_section(
        &self,
        section: inspect::Section,
        colors: Option<&ConsoleColors>,
    ) -> Element<'static, app::Message> {
        let collapsed = self.is_collapsed(section.name);
        let body = (!collapsed).then(|| render_blocks(section.blocks, colors));
        section_chrome(
            section.name,
            &section.summary,
            section.active,
            section.detail,
            collapsed,
            body,
        )
    }

    /// The collapsed CPU/video summary shown while the core runs before the
    /// first snapshot lands. Fed by the lightweight [`RunningStatus`].
    pub fn running_summary(
        &self,
        status: Option<&RunningStatus>,
    ) -> Element<'static, app::Message> {
        let (cpu_summary, video_label, video_summary) = match status {
            Some(status) => (
                format!("pc {:04X} · sp {:04X}", status.pc, status.sp),
                status.video_label,
                status.video_summary.clone(),
            ),
            None => (String::from("running"), "PPU", String::from("running")),
        };

        column![
            section_chrome("CPU", &cpu_summary, Some(true), None, true, None),
            section_chrome(video_label, &video_summary, Some(true), None, true, None),
        ]
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .height(Fill)
        .spacing(s())
        .into()
    }
}

// --- Section chrome ----------------------------------------------------------

fn section_chrome(
    name: &'static str,
    summary: &str,
    active: Option<bool>,
    detail: Option<inspect::Detail>,
    collapsed: bool,
    body: Option<Element<'static, app::Message>>,
) -> Element<'static, app::Message> {
    let mut header_left = Vec::new();

    if let Some(active) = active {
        header_left.push(pip(active, palette::GREEN));
    }

    header_left.push(
        text(name)
            .font(fonts::title())
            .size(13.0)
            .color(palette::MUTED)
            .into(),
    );

    // Right side: collapsed summary, or an expanded accent detail if present.
    let header_right: Element<'static, app::Message> = if collapsed {
        text(summary.to_owned())
            .font(fonts::monospace())
            .size(DETAIL)
            .color(palette::OVERLAY0)
            .into()
    } else if let Some(detail) = detail {
        text(detail.text)
            .font(fonts::monospace())
            .size(DETAIL)
            .color(tone_color(detail.tone))
            .into()
    } else {
        Space::new().into()
    };

    let header = button(
        container(
            iced::widget::row(header_left)
                .push(Space::new().width(Length::Fill))
                .push(header_right)
                .spacing(xs())
                .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .padding([xs(), s()])
        .style(section_header_style),
    )
    .on_press(Message::ToggleSection(name.to_string()).into())
    .padding(0)
    .style(|_, _| button::Style::default())
    .width(Length::Fill);

    let mut content = column![header].width(Length::Fill);
    if let Some(body) = body {
        content = content.push(body);
    }

    container(content)
        .width(Length::Fill)
        .style(section_style)
        .into()
}

/// The palette accent for a detail's semantic tone — the Game Boy PPU mode
/// colours, now serving every core's coloured section detail.
fn tone_color(tone: inspect::Tone) -> Color {
    match tone {
        inspect::Tone::Neutral => palette::MUTED,
        inspect::Tone::Idle => palette::BLUE,
        inspect::Tone::Active => palette::GREEN,
        inspect::Tone::Scanning => palette::YELLOW,
        inspect::Tone::Rendering => palette::PEACH,
        inspect::Tone::Pending => palette::YELLOW,
    }
}

// --- Blocks ------------------------------------------------------------------

fn render_blocks(
    blocks: Vec<inspect::SectionBlock>,
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    let mut body = column![].padding(s()).spacing(s());
    for block in blocks {
        body = body.push(render_block(block, colors));
    }
    body.into()
}

fn render_block(
    block: inspect::SectionBlock,
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    use inspect::SectionBlock::*;
    match block {
        Registers(group) => registers_block(&group),
        Pairs(pairs) => pairs_block(&pairs),
        Pointers(pointers) => pointers_block(&pointers),
        Table(table) => bit_table(&table),
        Rows(rows) => rows_block(&rows),
        Sweeps(sweeps) => sweeps_block(&sweeps),
        Swatches(rows) => swatches_block(&rows, colors),
        Rule => rule::horizontal(1).into(),
    }
}

/// Wrap an element in a hover tooltip carrying its one-line help, if any. The
/// tooltip is an overlay, so it never disturbs the sidebar's layout.
fn with_help(
    element: Element<'static, app::Message>,
    help: Option<&'static str>,
) -> Element<'static, app::Message> {
    match help {
        Some(text_help) => tooltip(
            element,
            container(text(text_help).font(fonts::monospace()).size(LABEL)).padding([2.0, s()]),
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into(),
        None => element,
    }
}

// --- Pointers ----------------------------------------------------------------

fn pointers_block(pointers: &[inspect::Pointer]) -> Element<'static, app::Message> {
    let mut line = iced::widget::row![].spacing(s()).align_y(Vertical::Center);
    for pointer in pointers {
        line = line.push(pointer_item(pointer));
    }
    line.into()
}

fn pointer_item(pointer: &inspect::Pointer) -> Element<'static, app::Message> {
    let register = &pointer.register;
    // An inactive pointer (a halted CPU's pc) is dimmed and annotated.
    let inactive = pointer.active == Some(false);
    let value_color = if inactive {
        palette::OVERLAY0
    } else {
        palette::PURPLE
    };

    let display: Element<'static, app::Message> = iced::widget::row![
        text(register.name.to_owned())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(hex(register.value, register.bits))
            .font(fonts::monospace())
            .size(20.0)
            .color(value_color),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into();

    if inactive {
        tooltip(
            display,
            container(text("halted").font(fonts::monospace()).size(REG)).padding([2.0, s()]),
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into()
    } else {
        with_help(display, register.help)
    }
}

// --- Register pairs ----------------------------------------------------------

fn pairs_block(pairs: &[inspect::RegisterPair]) -> Element<'static, app::Message> {
    let mut stack = column![].spacing(s());
    for pair in pairs {
        stack = stack.push(pair_row(pair));
    }
    stack.into()
}

fn pair_row(pair: &inspect::RegisterPair) -> Element<'static, app::Message> {
    let combined = compound(pair);
    let line = if let inspect::ValueStyle::Flags(names) = pair.low.style {
        // The low half is a flags register (SM83 `f`): its slot stays empty and
        // the flags render as chips beside the combined value.
        iced::widget::row![
            container(register8(&pair.high)).width(Length::Fixed(REG8_WIDTH)),
            container("").width(Length::Fixed(REG8_WIDTH)),
            combined,
            flag_chips(pair.low.value, names),
        ]
    } else {
        iced::widget::row![
            container(register8(&pair.high)).width(Length::Fixed(REG8_WIDTH)),
            container(register8(&pair.low)).width(Length::Fixed(REG8_WIDTH)),
            combined,
        ]
    };
    line.spacing(s()).align_y(Vertical::Center).into()
}

fn register8(register: &inspect::Register) -> Element<'static, app::Message> {
    let display = iced::widget::row![
        text(register.name.to_owned())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(hex(register.value, register.bits))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::TEXT),
    ]
    .spacing(s())
    .into();
    with_help(display, register.help)
}

fn compound(pair: &inspect::RegisterPair) -> Element<'static, app::Message> {
    let name = format!("{}{}", pair.high.name, pair.low.name);
    let bits = pair.high.bits + pair.low.bits;
    iced::widget::row![
        text(name)
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
        text(hex(pair.combined(), bits))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
    ]
    .spacing(s())
    .into()
}

// --- Flat register file ------------------------------------------------------

fn registers_block(group: &inspect::RegisterGroup) -> Element<'static, app::Message> {
    let mut stack = column![].spacing(s());
    for register in &group.registers {
        stack = stack.push(register_row(register));
    }
    stack.into()
}

fn register_row(register: &inspect::Register) -> Element<'static, app::Message> {
    let value: Element<'static, app::Message> = match register.style {
        inspect::ValueStyle::Flags(names) => flag_chips(register.value, names),
        _ => with_help(
            text(scalar(register))
                .font(fonts::monospace())
                .size(REG)
                .color(palette::TEXT)
                .into(),
            register.help,
        ),
    };
    iced::widget::row![
        container(
            text(register.name.to_owned())
                .font(fonts::monospace())
                .size(REG)
                .color(palette::MUTED),
        )
        .width(Length::Fixed(REG8_WIDTH)),
        value,
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

// --- Flags -------------------------------------------------------------------

fn flag_chips(value: u32, names: &'static [inspect::FlagName]) -> Element<'static, app::Message> {
    let mut chips = iced::widget::row![].spacing(2.0);
    for flag in names {
        chips = chips.push(flag_char(
            flag.name,
            value & (1 << flag.bit) != 0,
            flag.help,
        ));
    }
    chips.into()
}

fn flag_char(name: &str, set: bool, help: Option<&'static str>) -> Element<'static, app::Message> {
    let (display, color) = if set {
        (name.to_uppercase(), palette::TEXT)
    } else {
        ("\u{00B7}".to_owned(), palette::SURFACE2) // middle dot
    };
    let chip = text(display)
        .font(fonts::monospace())
        .size(REG)
        .color(color)
        .into();
    with_help(chip, help)
}

// --- Label/value rows --------------------------------------------------------

/// Adjacent short rows coalesce onto one line to keep the video block as dense
/// as the hand-built sidebar was; a long value takes its own line.
fn rows_block(rows: &[inspect::Row]) -> Element<'static, app::Message> {
    let mut lines = column![].spacing(xs());
    for line in pack_rows(rows) {
        let mut packed = iced::widget::row![].spacing(s()).align_y(Vertical::Center);
        for row in line {
            packed = packed.push(row_item(row));
        }
        lines = lines.push(packed);
    }
    lines.into()
}

fn pack_rows(rows: &[inspect::Row]) -> Vec<Vec<&inspect::Row>> {
    let mut lines = Vec::new();
    let mut line: Vec<&inspect::Row> = Vec::new();
    let mut width = 0.0;
    for row in rows {
        let item = estimated_width(row);
        let added = if line.is_empty() { item } else { item + s() };
        if !line.is_empty() && width + added > ROW_BUDGET {
            lines.push(std::mem::take(&mut line));
            width = item;
        } else {
            width += added;
        }
        line.push(row);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn estimated_width(row: &inspect::Row) -> f32 {
    let text = 8.0 * (row.label.len() + row.value.len()) as f32;
    let pip = if row.active.is_some() { 16.0 } else { 0.0 };
    20.0 + text + pip
}

fn row_item(row: &inspect::Row) -> Element<'static, app::Message> {
    let display: Element<'static, app::Message> = match row.active {
        Some(active) => iced::widget::row![
            pip(active, palette::GREEN),
            text(row.label.clone())
                .font(fonts::monospace())
                .size(LABEL)
                .color(if active {
                    palette::TEXT
                } else {
                    palette::SURFACE2
                }),
        ]
        .spacing(xs())
        .align_y(Vertical::Center)
        .into(),
        None => iced::widget::row![
            text(row.label.clone())
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::MUTED),
            text(row.value.clone())
                .font(fonts::monospace())
                .size(REG)
                .color(palette::TEXT),
        ]
        .spacing(xs())
        .align_y(Vertical::Center)
        .into(),
    };
    with_help(display, row.help)
}

// --- Period sweeps -----------------------------------------------------------

fn sweeps_block(sweeps: &[inspect::Sweep]) -> Element<'static, app::Message> {
    let mut stack = column![].spacing(xs());
    for sweep in sweeps {
        stack = stack.push(sweep_row(sweep));
    }
    stack.into()
}

fn sweep_row(sweep: &inspect::Sweep) -> Element<'static, app::Message> {
    let label = container(
        text(sweep.label)
            .font(fonts::monospace())
            .size(LABEL)
            .color(palette::MUTED),
    )
    .width(Length::Fixed(SWATCH_LABEL_WIDTH));

    let value = text(format!("{}/{}", sweep.value, sweep.end))
        .font(fonts::monospace())
        .size(LABEL)
        .color(palette::TEXT);

    let line: Element<'static, app::Message> = iced::widget::row![label, sweep_bar(sweep), value]
        .spacing(s())
        .align_y(Vertical::Center)
        .into();

    // The current zone name joins the help text in the hover tooltip.
    let zone = sweep.zone_at(sweep.value).map(|z| z.name);
    let tip = match (zone, sweep.help) {
        (Some(zone), Some(help)) => Some(format!("{zone} — {help}")),
        (Some(zone), None) => Some(zone.to_owned()),
        (None, Some(help)) => Some(help.to_owned()),
        (None, None) => None,
    };
    match tip {
        Some(tip) => tooltip(
            line,
            container(text(tip).font(fonts::monospace()).size(LABEL)).padding([2.0, s()]),
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into(),
        None => line,
    }
}

/// The number line: proportional zone segments with a position marker overlaid
/// at the value.
fn sweep_bar(sweep: &inspect::Sweep) -> Element<'static, app::Message> {
    let end = sweep.end.max(1) as f32;
    let marker_x = (sweep.value.min(sweep.end) as f32 / end) * SWEEP_BAR_WIDTH;

    let mut segments = iced::widget::row![];
    if sweep.zones.is_empty() {
        segments = segments.push(bar_segment(SWEEP_BAR_WIDTH, palette::SURFACE2));
    } else {
        let mut prev_end = 0u32;
        for zone in &sweep.zones {
            let width = (zone.end.saturating_sub(prev_end) as f32 / end) * SWEEP_BAR_WIDTH;
            segments = segments.push(bar_segment(width, tone_color(zone.tone)));
            prev_end = zone.end;
        }
    }

    let marker = iced::widget::row![
        Space::new().width(Length::Fixed(
            (marker_x - SWEEP_MARKER_WIDTH / 2.0).max(0.0)
        )),
        container(Space::new())
            .width(Length::Fixed(SWEEP_MARKER_WIDTH))
            .height(Length::Fixed(SWEEP_BAR_HEIGHT))
            .style(|_: &iced::Theme| container::Style {
                background: Some(Background::Color(palette::TEXT)),
                ..Default::default()
            }),
    ];

    iced::widget::stack![
        container(segments)
            .width(Length::Fixed(SWEEP_BAR_WIDTH))
            .height(Length::Fixed(SWEEP_BAR_HEIGHT)),
        marker,
    ]
    .width(Length::Fixed(SWEEP_BAR_WIDTH))
    .into()
}

fn bar_segment(width: f32, color: Color) -> Element<'static, app::Message> {
    container(Space::new())
        .width(Length::Fixed(width))
        .height(Length::Fixed(SWEEP_BAR_HEIGHT))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            ..Default::default()
        })
        .into()
}

// --- Bit table ---------------------------------------------------------------

fn bit_table(table: &inspect::BitTable) -> Element<'static, app::Message> {
    let mut columns = iced::widget::row![].spacing(xs()).align_y(Vertical::Top);

    // Leftmost column: the corner flag over each row's name.
    let corner: Element<'static, app::Message> = match &table.corner {
        Some(flag) => flag_badge(flag.name, flag.active),
        None => cell(Space::new().into()),
    };
    let mut labels = column![corner].spacing(s());
    for row in &table.rows {
        labels = labels.push(cell(
            text(row.name.to_owned())
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::MUTED)
                .into(),
        ));
    }
    columns = columns.push(labels);

    for (index, column) in table.columns.iter().enumerate() {
        let mut col = column![cell(column_header(column))]
            .spacing(s())
            .align_x(iced::alignment::Horizontal::Center);
        for row in &table.rows {
            let lit = row.bits.get(index).copied().unwrap_or(false);
            col = col.push(cell(pip(lit, pip_tone_color(row.tone))));
        }
        columns = columns.push(col);
    }

    columns.into()
}

/// A column heading: the concept's shared icon when the column names one (with
/// its name as a tooltip), else the column name as text.
fn column_header(column: &inspect::BitColumn) -> Element<'static, app::Message> {
    match column.concept.map(concept_icon) {
        Some(icon) => tooltip(
            icons::m_muted(icon),
            container(
                text(column.name.to_owned())
                    .font(fonts::monospace())
                    .size(HEADER),
            )
            .padding([2.0, s()]),
            tooltip::Position::Top,
        )
        .style(tooltip_style)
        .into(),
        None => text(column.name.to_owned())
            .font(fonts::monospace())
            .size(HEADER)
            .color(palette::MUTED)
            .into(),
    }
}

fn concept_icon(concept: inspect::Concept) -> Icon {
    match concept {
        inspect::Concept::VBlank => Icon::Monitor,
        inspect::Concept::VideoStatus => Icon::Eye,
        inspect::Concept::Timer => Icon::Clock,
        inspect::Concept::Serial => Icon::Wifi,
        inspect::Concept::Input => Icon::Gamepad,
    }
}

/// The pip colour for a bit-table row: a pending mask (interrupt-flag bits)
/// reads yellow, an ordinary enabled/status mask green.
fn pip_tone_color(tone: inspect::Tone) -> Color {
    match tone {
        inspect::Tone::Pending => palette::YELLOW,
        _ => palette::GREEN,
    }
}

/// A fixed-height table cell so headers, names, and pips align across columns.
fn cell(content: Element<'static, app::Message>) -> Element<'static, app::Message> {
    container(content)
        .height(Length::Fixed(CELL_HEIGHT))
        .center_y(Length::Fixed(CELL_HEIGHT))
        .into()
}

fn flag_badge(name: &str, active: bool) -> Element<'static, app::Message> {
    let text_color = if active {
        palette::GREEN
    } else {
        palette::SURFACE2
    };
    let bg = active.then(|| {
        Background::Color(Color::from_rgba(
            0xa6 as f32 / 255.0,
            0xe3 as f32 / 255.0,
            0xa1 as f32 / 255.0,
            0.12,
        ))
    });

    container(
        text(name.to_owned())
            .font(fonts::monospace())
            .size(LABEL)
            .color(text_color),
    )
    .padding([2.0, 4.0])
    .center_y(Length::Fixed(CELL_HEIGHT))
    .style(move |_: &iced::Theme| container::Style {
        background: bg,
        border: Border::default().rounded(4.0),
        ..Default::default()
    })
    .into()
}

// --- Palette swatches --------------------------------------------------------

fn swatches_block(
    rows: &[inspect::SwatchRow],
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    let mut stack = column![].spacing(xs());
    for row in rows {
        stack = stack.push(swatch_row(row, colors));
    }
    stack.into()
}

fn swatch_row(
    row: &inspect::SwatchRow,
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    match row {
        inspect::SwatchRow::Shades { label, packed } => {
            let palette = match colors {
                Some(ConsoleColors::Dmg { palette }) => *palette,
                _ => Palette::CLASSIC,
            };
            let map = PaletteMap(*packed);
            let swatches: Vec<Color> = (0..4)
                .map(|i| iced_color(map.color(PaletteIndex(i), &palette)))
                .collect();
            swatch_line(label, swatches, Some(format!("{:02X}", packed)))
        }
        inspect::SwatchRow::Colors { label, colors } => {
            let swatches: Vec<Color> = colors.iter().map(|c| iced_color(*c)).collect();
            swatch_line(label, swatches, None)
        }
    }
}

fn swatch_line(
    label: &str,
    swatches: Vec<Color>,
    trailing: Option<String>,
) -> Element<'static, app::Message> {
    let mut cells = iced::widget::row![].spacing(2.0);
    for color in swatches {
        cells = cells.push(color_swatch(color));
    }

    let mut line = iced::widget::row![
        container(
            text(label.to_owned())
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::MUTED),
        )
        .width(Length::Fixed(SWATCH_LABEL_WIDTH)),
        cells,
    ]
    .spacing(s())
    .align_y(Vertical::Center);

    if let Some(trailing) = trailing {
        line = line.push(
            text(trailing)
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::OVERLAY0),
        );
    }

    line.into()
}

fn color_swatch(color: Color) -> Element<'static, app::Message> {
    container(Space::new())
        .width(14.0)
        .height(14.0)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border::default()
                .rounded(2.0)
                .width(1.0)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)),
            ..Default::default()
        })
        .into()
}

// --- Shared widgets ----------------------------------------------------------

/// A small round activity indicator: filled when active, hollow when not.
pub fn pip(active: bool, active_color: Color) -> Element<'static, app::Message> {
    let (bg, border_color) = if active {
        (Some(Background::Color(active_color)), active_color)
    } else {
        (None, palette::SURFACE2)
    };

    container(Space::new())
        .width(10.0)
        .height(10.0)
        .style(move |_: &iced::Theme| container::Style {
            background: bg,
            border: Border::default()
                .rounded(5.0)
                .width(1.5)
                .color(border_color),
            ..Default::default()
        })
        .into()
}

fn hex(value: u32, bits: u8) -> String {
    let width = (bits as usize).div_ceil(4).max(1);
    format!("{value:0width$X}")
}

fn scalar(register: &inspect::Register) -> String {
    match register.style {
        inspect::ValueStyle::Hex => hex(register.value, register.bits),
        inspect::ValueStyle::Dec => register.value.to_string(),
        inspect::ValueStyle::Bool => if register.value != 0 { "true" } else { "false" }.to_owned(),
        inspect::ValueStyle::Flags(_) => String::new(),
    }
}

// --- Styles ------------------------------------------------------------------

fn section_style(theme: &iced::Theme) -> container::Style {
    let pal = theme.extended_palette();
    container::Style {
        background: Some(pal.background.base.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
        ..Default::default()
    }
}

fn section_header_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.03).into()),
        ..Default::default()
    }
}

pub fn tooltip_style(theme: &iced::Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(palette.background.strong.color),
        ..Default::default()
    }
}
