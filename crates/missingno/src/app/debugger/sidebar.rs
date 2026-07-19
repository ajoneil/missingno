use std::collections::HashSet;

use iced::{
    Background, Border, Color, Element,
    Length::{self, Fill},
    alignment::Vertical,
    widget::{Space, button, column, container, rule, scrollable, text, tooltip},
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
/// A pixel-strip cell's width and height; height runs taller so the strip reads
/// as a bold pixel row. A strip too wide to sit beside its label (the
/// playfield's 20 cells) stacks beneath it instead.
const PIXEL_CELL_W: f32 = 9.0;
const PIXEL_CELL_H: f32 = 13.0;
/// A bit table cell's height, so its header, rows, and pips align across
/// columns.
const CELL_HEIGHT: f32 = 16.0;
/// A pair-matrix cell's width and height, so its column headers and triangular
/// pips align with room to breathe.
const PAIR_CELL_W: f32 = 20.0;
const PAIR_CELL_H: f32 = 18.0;

/// Content width available to a packed row block, inside the section body's
/// padding. Adjacent short rows coalesce onto one line up to this budget.
const ROW_BUDGET: f32 = 236.0;

/// The number-line width for a period sweep and its bar height — sized to sit on
/// one sidebar row beside a label and a value. A small triangle glyph sits below
/// the bar pointing up at the value; it rides in a fixed-width centred cell so
/// its own horizontal centre — not its glyph advance — lands on the position.
const SWEEP_BAR_WIDTH: f32 = 96.0;
const SWEEP_BAR_HEIGHT: f32 = 6.0;
const SWEEP_MARKER_SIZE: f32 = 9.0;
/// The marker glyph's fixed cell width; the glyph is centred inside it, so the
/// cell centre is the alignment anchor regardless of the glyph's true advance.
const SWEEP_MARKER_GLYPH_W: f32 = 7.0;

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
        let mut stack = column![].width(Length::Fixed(SIDEBAR_WIDTH)).spacing(s());
        for section in sections {
            stack = stack.push(self.render_section(section, colors));
        }
        scroll_sidebar(stack.into())
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

        let summary = column![
            section_chrome("CPU", &cpu_summary, Some(true), None, true, None),
            section_chrome(video_label, &video_summary, Some(true), None, true, None),
        ]
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .spacing(s());
        scroll_sidebar(summary.into())
    }
}

/// Wrap the sidebar sections in a vertical scrollable so they scroll on
/// overflow, with a hidden scrollbar so no gutter eats the fixed width.
fn scroll_sidebar(content: Element<'static, app::Message>) -> Element<'static, app::Message> {
    scrollable(content)
        .direction(iced::widget::scrollable::Direction::Vertical(
            iced::widget::scrollable::Scrollbar::new()
                .width(0)
                .scroller_width(0),
        ))
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .height(Fill)
        .into()
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
pub(crate) fn tone_color(tone: inspect::Tone) -> Color {
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
        Relations(matrix) => pair_matrix(&matrix),
        Rows(rows) => rows_block(&rows),
        Sweeps(sweeps) => sweeps_block(&sweeps),
        Swatches(rows) => swatches_block(&rows, colors),
        Pixels(strips) => pixels_block(&strips, colors),
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

/// The number line: proportional zone segments, with a small triangle below the
/// bar pointing up at the value.
fn sweep_bar(sweep: &inspect::Sweep) -> Element<'static, app::Message> {
    let end = sweep.end.max(1) as f32;
    let position = marker_x(sweep.value, sweep.end, SWEEP_BAR_WIDTH);

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

    // The marker line overhangs the bar by half a glyph on each side, so the
    // glyph's centre can sit exactly on the position even at the ends; the bar
    // is inset by that margin to share the same coordinate line.
    let margin = SWEEP_MARKER_GLYPH_W / 2.0;
    let marker = iced::widget::row![
        Space::new().width(Length::Fixed(position)),
        container(
            text("\u{25B2}") // ▲
                .font(fonts::monospace())
                .size(SWEEP_MARKER_SIZE)
                .color(palette::TEXT),
        )
        .center_x(Length::Fixed(SWEEP_MARKER_GLYPH_W)),
    ];

    column![
        iced::widget::row![
            Space::new().width(Length::Fixed(margin)),
            container(segments)
                .width(Length::Fixed(SWEEP_BAR_WIDTH))
                .height(Length::Fixed(SWEEP_BAR_HEIGHT)),
        ],
        marker,
    ]
    .width(Length::Fixed(SWEEP_BAR_WIDTH + SWEEP_MARKER_GLYPH_W))
    .spacing(1.0)
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

/// The value's proportional position along the bar, in pixels from the left.
fn marker_x(value: u32, end: u32, bar_width: f32) -> f32 {
    let span = end.max(1) as f32;
    (value.min(end) as f32 / span) * bar_width
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

// --- Pair matrix -------------------------------------------------------------

/// A symmetric relation as a lower-triangular pip grid: object labels down the
/// left (every entity past the first), object headers across the top (every
/// entity before the last), and a pip where a row meets a column below its
/// diagonal — the cell for that unordered pair. The empty upper triangle is left
/// as blank space.
fn pair_matrix(matrix: &inspect::PairMatrix) -> Element<'static, app::Message> {
    let n = matrix.entities.len();
    let mut grid = column![];

    // Header line: a blank corner over the label column, then one header per
    // entity that can be the lower member of a pair.
    let mut header = iced::widget::row![pair_slot(Space::new().into())];
    for &name in &matrix.entities[..n.saturating_sub(1)] {
        header = header.push(pair_rule_v());
        header = header.push(pair_slot(
            text(name.to_owned())
                .font(fonts::monospace())
                .size(HEADER)
                .color(palette::MUTED)
                .into(),
        ));
    }
    grid = grid.push(header);

    // One line per row entity, widest first so each column's pips sit close
    // under their header, with a faint rule above each line stepping narrower
    // as the triangle tapers, and a vertical rule between every column.
    for row in (1..n).rev() {
        grid = grid.push(pair_rule_h(1 + row));
        let mut line = iced::widget::row![pair_slot(
            text(matrix.entities[row].to_owned())
                .font(fonts::monospace())
                .size(LABEL)
                .color(palette::MUTED)
                .into(),
        )];
        for col in 0..row {
            let cell = matrix.cell(col, row);
            line = line.push(pair_rule_v());
            line = line.push(pair_slot(with_help(
                pip(cell.set, palette::GREEN),
                cell.help,
            )));
        }
        grid = grid.push(line);
    }

    container(grid).padding([xs(), 0.0]).into()
}

/// A fixed-size pair-matrix cell, so headers and pips align across the triangle.
fn pair_slot(content: Element<'static, app::Message>) -> Element<'static, app::Message> {
    container(content)
        .width(Length::Fixed(PAIR_CELL_W))
        .height(Length::Fixed(PAIR_CELL_H))
        .center_x(Length::Fixed(PAIR_CELL_W))
        .center_y(Length::Fixed(PAIR_CELL_H))
        .into()
}

/// A faint horizontal rule spanning `slots` matrix columns and the vertical
/// rules between them, guiding the eye along each triangle row.
fn pair_rule_h(slots: usize) -> Element<'static, app::Message> {
    let width = slots as f32 * PAIR_CELL_W + slots.saturating_sub(1) as f32;
    container(Space::new())
        .width(Length::Fixed(width))
        .height(Length::Fixed(1.0))
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(pair_rule_color())),
            ..Default::default()
        })
        .into()
}

/// A faint vertical rule between matrix columns.
fn pair_rule_v() -> Element<'static, app::Message> {
    container(Space::new())
        .width(Length::Fixed(1.0))
        .height(Length::Fixed(PAIR_CELL_H))
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(pair_rule_color())),
            ..Default::default()
        })
        .into()
}

fn pair_rule_color() -> Color {
    Color {
        a: 0.35,
        ..palette::SURFACE2
    }
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

// --- Pixel strips ------------------------------------------------------------

fn pixels_block(
    strips: &[inspect::PixelStrip],
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    let mut stack = column![].spacing(xs());
    for strip in strips {
        stack = stack.push(pixel_strip_row(strip, colors));
    }
    stack.into()
}

fn pixel_strip_row(
    strip: &inspect::PixelStrip,
    colors: Option<&ConsoleColors>,
) -> Element<'static, app::Message> {
    let (label, help, cells): (String, Option<&'static str>, Vec<Option<Color>>) = match strip {
        inspect::PixelStrip::Shades { label, cells, help } => {
            let palette = match colors {
                Some(ConsoleColors::Dmg { palette }) => *palette,
                _ => Palette::CLASSIC,
            };
            let cells = cells
                .iter()
                .map(|c| c.map(|shade| iced_color(palette.color(PaletteIndex(shade)))))
                .collect();
            (label.to_string(), *help, cells)
        }
        inspect::PixelStrip::Colors { label, cells, help } => {
            let cells = cells.iter().map(|c| c.map(iced_color)).collect();
            (label.clone(), *help, cells)
        }
        inspect::PixelStrip::Bits { label, cells, help } => {
            let cells = cells.iter().map(|&b| b.then_some(palette::TEXT)).collect();
            (label.to_string(), *help, cells)
        }
    };

    let cell_count = cells.len();
    let mut strip_cells = iced::widget::row![].spacing(1.0);
    for cell in cells {
        strip_cells = strip_cells.push(pixel_cell(cell));
    }
    // One 1px frame around the whole strip, its fill also showing through the
    // 1px inter-cell gaps as shared separators between neighbouring cells.
    let strip = container(strip_cells)
        .padding(1.0)
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(palette::SURFACE2)),
            border: Border::default()
                .rounded(2.0)
                .width(1.0)
                .color(palette::SURFACE2),
            ..Default::default()
        });

    let label_text = text(label)
        .font(fonts::monospace())
        .size(LABEL)
        .color(palette::MUTED);

    // A strip too wide to sit beside its label stacks beneath it, using the
    // full row budget.
    let line: Element<'static, app::Message> = if strip_width(cell_count) > STRIP_BESIDE_BUDGET {
        column![label_text, strip].spacing(2.0).into()
    } else {
        iced::widget::row![
            container(label_text).width(Length::Fixed(SWATCH_LABEL_WIDTH)),
            strip,
        ]
        .spacing(s())
        .align_y(Vertical::Center)
        .into()
    };

    with_help(line, help)
}

/// Content width left for a strip sitting beside its fixed-width label.
const STRIP_BESIDE_BUDGET: f32 = ROW_BUDGET - SWATCH_LABEL_WIDTH - 8.0;

/// A strip's rendered width: its cells, the 1px separators between them, and
/// the 1px frame + 1px padding on each side.
fn strip_width(cells: usize) -> f32 {
    cells as f32 * PIXEL_CELL_W + cells.saturating_sub(1) as f32 + 4.0
}

/// One pixel-strip cell, separated from its neighbours by the strip frame's 1px
/// grid lines: a solid fill when lit, or the page background when the pixel is
/// off (a 0 pattern bit, or a hardware-transparent object colour 0) — an off
/// cell reads as absence, while a lit black pixel fills darker than the page.
fn pixel_cell(color: Option<Color>) -> Element<'static, app::Message> {
    container(Space::new())
        .width(PIXEL_CELL_W)
        .height(PIXEL_CELL_H)
        .style(move |theme: &iced::Theme| container::Style {
            background: Some(Background::Color(match color {
                Some(fill) => fill,
                None => theme.extended_palette().background.base.color,
            })),
            ..Default::default()
        })
        .into()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_glyph_centres_on_value() {
        let end = 8;
        let bar = SWEEP_BAR_WIDTH;
        let g = SWEEP_MARKER_GLYPH_W;
        // The bar is inset by g/2 on the marker line, so the bar's left edge
        // sits at g/2; a left pad of marker_x centres the glyph cell at
        // marker_x + g/2 — the value's position in bar coordinates.
        let bar_left = g / 2.0;
        let centre = |value| marker_x(value, end, bar) + g / 2.0;

        // value = 0: the tip sits exactly on the bar's left edge.
        assert!((centre(0) - bar_left).abs() < 1e-4);

        // Mid-bar: exactly on the value's position.
        assert!((centre(4) - (bar_left + bar / 2.0)).abs() < 1e-4);

        // value = end: exactly on the bar's right edge.
        assert!((centre(end) - (bar_left + bar)).abs() < 1e-4);

        // The glyph never needs to leave the margined line.
        assert!(centre(0) - g / 2.0 >= 0.0);
        assert!(centre(end) + g / 2.0 <= bar + g + 1e-4);
    }
}
