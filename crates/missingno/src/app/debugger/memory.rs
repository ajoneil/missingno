//! A generic memory viewer pane, registered for every family. While paused it
//! reads live through the seam's `memory_regions()` + `peek()`, showing a
//! region picker and a scrollable hex/ASCII grid. While the core runs on the
//! emulation thread the same browser renders from the interest window the emu
//! thread peeks each vblank for the pane's current view: scrolling emits a new
//! interest and the bytes catch up next vblank, with a placeholder row where
//! the published window doesn't yet cover the selection. A family with no
//! region map falls back to the PC-anchored snapshot window.

use iced::widget::text::Span;
use iced::widget::{
    Column, button, column, container, pane_grid, pick_list, rich_text, row, text, text_input,
};
use iced::{Color, Element, Length};

use missingno_core::inspect::{MemoryRegion, MemoryWindow};

use crate::app;
use crate::app::debugger::panes::{self, DebuggerPane, Pane, PaneContext, PaneMessage};
use crate::app::system::SystemDebugger;
use crate::app::ui::{fonts, palette, sizes::s};

/// Bytes per grid row.
const BYTES_PER_ROW: u32 = 16;
/// Rows shown at once; the paused peek copies exactly this window.
const VISIBLE_ROWS: u32 = 16;
/// One screen's worth of bytes — the bounded span copied per render.
const VISIBLE_BYTES: u32 = BYTES_PER_ROW * VISIBLE_ROWS;

/// The memory pane's current view, owned by the pane and consulted by the
/// context builder to copy the right bytes.
#[derive(Clone, Copy)]
pub struct MemorySelection {
    /// Index into the core's region list.
    pub region: usize,
    /// Byte offset within that region where the visible window starts.
    pub offset: u32,
}

/// The span the running pane wants peeked each vblank: a base address and a
/// length. The session engine caps the length before reading through the seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryInterest {
    pub start: u32,
    pub len: u32,
}

/// What the memory panes render from this frame. All open instances share one
/// of these; each picks its own bytes by matching its window's base, so several
/// memory panes on different regions coexist. Copied into the pane context each
/// frame so a pane never borrows the core.
#[derive(Clone, Copy)]
pub enum MemoryPaneData<'b> {
    /// The region browser fed by one window per open memory pane, each matched
    /// by its base — the paused live peeks and the running vblank interest
    /// windows alike. A pane with no matching window yet shows placeholders.
    Browse {
        regions: &'b [MemoryRegion],
        windows: &'b [MemoryWindow],
    },
    /// Running fallback for a family without a region map: the PC-anchored
    /// snapshot window, unscrollable.
    Window(&'b MemoryWindow),
}

impl<'b> MemoryPaneData<'b> {
    /// The paused view over one live peek window per open memory pane.
    pub fn paused(regions: &'b [MemoryRegion], windows: &'b [MemoryWindow]) -> Self {
        Self::Browse { regions, windows }
    }

    /// The running browser fed by the vblank interest windows.
    pub fn running_browse(regions: &'b [MemoryRegion], windows: &'b [MemoryWindow]) -> Self {
        Self::Browse { regions, windows }
    }

    /// The running PC-anchored fallback for a family with no region map.
    pub fn running_window(window: &'b MemoryWindow) -> Self {
        Self::Window(window)
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    SelectRegion(usize),
    SetOffset(u32),
    JumpInput(String),
    Jump,
    /// The core's region map, pushed in so the pane's jump-to-address can
    /// resolve while the core runs on the emu thread.
    SetRegions(Vec<MemoryRegion>),
}

/// The largest row-snapped offset that still fills the window from a region of
/// `len` bytes — where the last page begins.
fn max_offset(len: u32) -> u32 {
    len.saturating_sub(VISIBLE_BYTES).div_ceil(BYTES_PER_ROW) * BYTES_PER_ROW
}

/// Clamp a selection to a region and resolve the visible window:
/// `(clamped_offset, base_address, visible_len)`, row-snapped.
fn window_range(region_start: u32, region_len: u32, offset: u32) -> (u32, u32, u32) {
    let offset = (offset / BYTES_PER_ROW * BYTES_PER_ROW).min(max_offset(region_len));
    let base = region_start.saturating_add(offset);
    let visible = VISIBLE_BYTES.min(region_len.saturating_sub(offset));
    (offset, base, visible)
}

/// The interest span for a selection over a region list — the exact window the
/// pane shows, so the emu thread peeks the same bytes. `None` with no regions.
pub fn interest_for(
    regions: &[MemoryRegion],
    selection: MemorySelection,
) -> Option<MemoryInterest> {
    let selected = selection.region.min(regions.len().checked_sub(1)?);
    let region = regions[selected];
    let (_, base, visible) = window_range(region.start, region.len, selection.offset);
    Some(MemoryInterest {
        start: base,
        len: visible,
    })
}

/// The union of interests across every open memory pane — the spans the emu
/// thread peeks each vblank, one window per interest.
pub fn interests_for(
    regions: &[MemoryRegion],
    selections: &[MemorySelection],
) -> Vec<MemoryInterest> {
    selections
        .iter()
        .filter_map(|&selection| interest_for(regions, selection))
        .collect()
}

/// Resolve a typed hex address to the region containing it and a row-snapped,
/// clamped offset within it. `None` for unparseable input or an address in no
/// region.
fn resolve_jump(regions: &[MemoryRegion], input: &str) -> Option<(usize, u32)> {
    let trimmed = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches('$');
    let address = u32::from_str_radix(trimmed, 16).ok()?;
    let (index, region) = regions
        .iter()
        .enumerate()
        .find(|(_, r)| address >= r.start && address < r.start.saturating_add(r.len))?;
    let offset = (address - region.start) / BYTES_PER_ROW * BYTES_PER_ROW;
    Some((index, offset.min(max_offset(region.len))))
}

/// Copy the bytes one paused memory pane should show for `selection`, reading
/// side-effect-free through the seam. Bounded to one screen's worth; keyed by
/// `base` so the pane can match its own readout.
pub fn build_readout(
    core: &dyn SystemDebugger,
    regions: &[MemoryRegion],
    selection: MemorySelection,
) -> MemoryWindow {
    if regions.is_empty() {
        return MemoryWindow {
            base: 0,
            bytes: Vec::new(),
        };
    }
    let selected = selection.region.min(regions.len() - 1);
    let region = regions[selected];
    let (_, base, visible) = window_range(region.start, region.len, selection.offset);
    let bytes = (0..visible).map(|i| core.peek(base + i)).collect();
    MemoryWindow { base, bytes }
}

/// Hue span in degrees a byte value is mapped across — short of a full wheel so
/// 0x00 and 0xFF land on distinct hues rather than colliding at red.
const HUE_SPAN: f32 = 300.0;
/// Fixed saturation/lightness keeping the tints soft on the dark theme, so the
/// colouring reads as data-tinting rather than rainbow noise.
const CELL_SATURATION: f32 = 0.55;
const CELL_LIGHTNESS: f32 = 0.70;

/// The hue in degrees a byte maps to: a monotonic ramp across `HUE_SPAN`.
fn byte_hue(value: u8) -> f32 {
    value as f32 / u8::MAX as f32 * HUE_SPAN
}

/// The tint a byte's hex pair and its ASCII glyph share — equal bytes tint alike.
fn byte_color(value: u8) -> Color {
    let (r, g, b) = hsl_to_rgb(byte_hue(value), CELL_SATURATION, CELL_LIGHTNESS);
    Color::from_rgb(r, g, b)
}

/// HSL→RGB with hue in degrees and saturation/lightness in `0.0..=1.0`.
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (f32, f32, f32) {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let secondary = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let base = lightness - chroma / 2.0;
    (r + base, g + base, b + base)
}

/// The ASCII glyph for a cell and the colour it carries: a printable byte as
/// itself, any other published byte as `.`, both tinted by the byte's value; an
/// unpublished cell blanks with no tint.
fn ascii_cell(cell: Option<u8>) -> (char, Option<Color>) {
    match cell {
        Some(b) if (0x20..=0x7E).contains(&b) => (b as char, Some(byte_color(b))),
        Some(b) => ('.', Some(byte_color(b))),
        None => (' ', None),
    }
}

/// The coloured runs of one grid line: `$ADDR  XX XX .. XX  |ascii|`, with the
/// address and separators muted, each hex pair and its ASCII glyph tinted by the
/// byte's value. A `None` cell is a byte the running window hasn't published yet
/// — a muted `--` and a blank ASCII column so scrolling reads as updating rather
/// than as zeroes. Short final rows pad the hex columns.
fn row_spans(address: u32, cells: &[Option<u8>]) -> Vec<(String, Option<Color>)> {
    let mut spans = vec![(format!("${address:04X}  "), Some(palette::OVERLAY0))];
    for i in 0..BYTES_PER_ROW as usize {
        if i > 0 {
            spans.push((" ".to_owned(), None));
        }
        match cells.get(i) {
            Some(Some(byte)) => spans.push((format!("{byte:02X}"), Some(byte_color(*byte)))),
            Some(None) => spans.push(("--".to_owned(), Some(palette::MUTED))),
            None => spans.push(("  ".to_owned(), None)),
        }
    }
    spans.push(("  |".to_owned(), Some(palette::MUTED)));
    for &cell in cells {
        let (glyph, color) = ascii_cell(cell);
        spans.push((glyph.to_string(), color));
    }
    spans.push(("|".to_owned(), Some(palette::MUTED)));
    spans
}

pub struct MemoryPane {
    selection: MemorySelection,
    jump_input: String,
    /// Cached region map so jump-to-address resolves while the core is away.
    regions: Vec<MemoryRegion>,
}

impl MemoryPane {
    pub fn new() -> Self {
        Self {
            selection: MemorySelection {
                region: 0,
                offset: 0,
            },
            jump_input: String::new(),
            regions: Vec::new(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SelectRegion(region) => {
                self.selection.region = region;
                self.selection.offset = 0;
            }
            Message::SetOffset(offset) => self.selection.offset = offset,
            Message::JumpInput(input) => self.jump_input = input,
            Message::Jump => {
                if let Some((region, offset)) = resolve_jump(&self.regions, &self.jump_input) {
                    self.selection.region = region;
                    self.selection.offset = offset;
                    self.jump_input.clear();
                }
            }
            Message::SetRegions(regions) => self.regions = regions,
        }
    }

    fn grid(base: u32, cells: &[Option<u8>]) -> Element<'static, app::Message> {
        let rows = cells
            .chunks(BYTES_PER_ROW as usize)
            .enumerate()
            .map(|(i, row)| {
                let spans: Vec<Span<'static, &'static str>> =
                    row_spans(base + i as u32 * BYTES_PER_ROW, row)
                        .into_iter()
                        .map(|(text, color)| Span {
                            text: text.into(),
                            color,
                            ..Default::default()
                        })
                        .collect();
                rich_text(spans).font(fonts::monospace()).size(13.0).into()
            });
        Column::from_iter(rows).into()
    }

    /// The region browser shared by the paused and running views: a picker,
    /// scroll controls, a jump-to-address field, and the hex/ASCII grid — the
    /// grid fed from `source` so paused live bytes and the running interest
    /// window render identically.
    fn browser_view(
        &self,
        regions: &[MemoryRegion],
        window: Option<&MemoryWindow>,
        id: pane_grid::Pane,
    ) -> Element<'static, app::Message> {
        let selected = self.selection.region.min(regions.len() - 1);
        let region = regions[selected];
        let (offset, base, visible) = window_range(region.start, region.len, self.selection.offset);
        // A cell outside the matched window reads as an unpublished placeholder.
        let cells: Vec<Option<u8>> = (0..visible)
            .map(|i| window.and_then(|w| w.read(base + i)))
            .collect();

        let choices: Vec<RegionChoice> = regions
            .iter()
            .enumerate()
            .map(|(index, region)| RegionChoice::new(index, region))
            .collect();
        let picker = pick_list(
            choices,
            Some(RegionChoice::new(selected, &region)),
            move |choice| targeted(id, Message::SelectRegion(choice.index)),
        )
        .font(fonts::monospace())
        .text_size(13.0);

        let ceiling = max_offset(region.len);
        let page_up = scroll_button(
            "\u{21C8}",
            offset > 0,
            targeted(id, Message::SetOffset(offset.saturating_sub(VISIBLE_BYTES))),
        );
        let up = scroll_button(
            "\u{2191}",
            offset > 0,
            targeted(id, Message::SetOffset(offset.saturating_sub(BYTES_PER_ROW))),
        );
        let down = scroll_button(
            "\u{2193}",
            offset < ceiling,
            targeted(
                id,
                Message::SetOffset((offset + BYTES_PER_ROW).min(ceiling)),
            ),
        );
        let page_down = scroll_button(
            "\u{21CA}",
            offset < ceiling,
            targeted(
                id,
                Message::SetOffset((offset + VISIBLE_BYTES).min(ceiling)),
            ),
        );

        let jump = text_input("$addr", &self.jump_input)
            .font(fonts::monospace())
            .size(13.0)
            .width(Length::Fixed(96.0))
            .on_input(move |value| targeted(id, Message::JumpInput(value)))
            .on_submit(targeted(id, Message::Jump));

        let controls = row![picker, page_up, up, down, page_down, jump]
            .spacing(s())
            .align_y(iced::alignment::Vertical::Center);

        column![controls, Self::grid(base, &cells)]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The running fallback for a family with no region map: the PC-anchored
    /// snapshot window, with a note that browsing needs a pause.
    fn window_view(window: &MemoryWindow) -> Element<'static, app::Message> {
        let hint = text("Pause to browse full memory")
            .font(fonts::monospace())
            .size(11.0)
            .color(palette::MUTED);
        let cells: Vec<Option<u8>> = window.bytes.iter().map(|&b| Some(b)).collect();
        column![hint, Self::grid(window.base, &cells)]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The window base this pane shows for its selection — the key it matches
    /// its own readout or interest window by.
    fn base(&self, regions: &[MemoryRegion]) -> u32 {
        interest_for(regions, self.selection)
            .map(|interest| interest.start)
            .unwrap_or(0)
    }

    /// The title-bar detail: the selected region and its visible range.
    fn detail(&self, regions: &[MemoryRegion]) -> Option<Element<'static, app::Message>> {
        let selected = self.selection.region.min(regions.len().checked_sub(1)?);
        let region = regions[selected];
        let (_, base, visible) = window_range(region.start, region.len, self.selection.offset);
        let end = base + visible.saturating_sub(1);
        Some(
            text(format!("{} ${base:04X}-${end:04X}", region.name))
                .font(fonts::monospace())
                .size(11.0)
                .color(palette::MUTED)
                .into(),
        )
    }
}

/// Wrap a memory message for delivery to just this pane instance.
fn targeted(id: pane_grid::Pane, message: Message) -> app::Message {
    PaneMessage::Memory(message).to(id)
}

fn scroll_button(
    glyph: &'static str,
    enabled: bool,
    message: app::Message,
) -> Element<'static, app::Message> {
    let label = text(glyph).font(fonts::monospace()).size(13.0);
    let mut btn = button(label).style(button::text);
    if enabled {
        btn = btn.on_press(message);
    }
    btn.into()
}

/// A region option for the picker: its index plus a display of name and range.
#[derive(Clone, PartialEq)]
struct RegionChoice {
    index: usize,
    label: String,
}

impl RegionChoice {
    fn new(index: usize, region: &MemoryRegion) -> Self {
        let end = region.start + region.len.saturating_sub(1);
        Self {
            index,
            label: format!("{}  ${:04X}-${:04X}", region.name, region.start, end),
        }
    }
}

impl std::fmt::Display for RegionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl Pane for MemoryPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Memory
    }

    fn view<'a>(
        &'a self,
        ctx: Option<&PaneContext<'_>>,
        id: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        let Some(data) = ctx.and_then(|ctx| ctx.memory) else {
            return panes::running_placeholder("Memory", id);
        };
        let (body, detail) = match data {
            MemoryPaneData::Browse { regions, windows } => {
                if regions.is_empty() {
                    (no_map_body(), None)
                } else {
                    // Match this pane's own window by its base; a pane whose
                    // window hasn't arrived yet shows placeholders.
                    let base = self.base(regions);
                    let window = windows.iter().find(|window| window.base == base);
                    (self.browser_view(regions, window, id), self.detail(regions))
                }
            }
            MemoryPaneData::Window(window) => (Self::window_view(window), None),
        };
        let title = match detail {
            Some(detail) => panes::title_bar_with_detail("Memory", detail, id),
            None => panes::title_bar("Memory", id),
        };
        panes::pane(title, body)
    }

    fn on_message(&mut self, message: &PaneMessage) {
        if let PaneMessage::Memory(message) = message {
            self.update(message.clone());
        }
    }

    fn source_index(&self) -> Option<usize> {
        // Report the effective selection so a stale stored region — clamped at
        // render time — is not re-persisted into the saved layout.
        Some(self.effective_region())
    }

    fn set_source_index(&mut self, index: usize) {
        // Clamp against the known region map when we have one, so a restore or
        // instance-open past the end lands on a real region.
        self.selection.region = if self.regions.is_empty() {
            index
        } else {
            index.min(self.regions.len() - 1)
        };
        self.selection.offset = 0;
    }

    fn set_source_offset(&mut self, offset: u32) {
        self.selection.offset = offset;
    }

    fn source_offset(&self) -> Option<u32> {
        Some(self.selection.offset)
    }

    fn memory_selection(&self) -> Option<MemorySelection> {
        Some(self.selection)
    }
}

impl MemoryPane {
    /// The region actually shown: the stored selection clamped to the cached
    /// region map, or the stored value verbatim before any map has arrived.
    fn effective_region(&self) -> usize {
        match self.regions.len().checked_sub(1) {
            Some(last) => self.selection.region.min(last),
            None => self.selection.region,
        }
    }
}

fn no_map_body() -> Element<'static, app::Message> {
    container(
        text("No memory map")
            .font(fonts::monospace())
            .color(palette::MUTED),
    )
    .center(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(name: &'static str, start: u32, len: u32) -> MemoryRegion {
        MemoryRegion { name, start, len }
    }

    fn cells(bytes: &[u8]) -> Vec<Option<u8>> {
        bytes.iter().map(|&b| Some(b)).collect()
    }

    /// The plain text of a row, concatenating its span runs — the same string
    /// the pane used to build before it was split into coloured spans.
    fn row_text(address: u32, cells: &[Option<u8>]) -> String {
        row_spans(address, cells)
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    }

    #[test]
    fn ascii_cell_maps_printable_dots_the_rest_and_blanks_gaps() {
        assert_eq!(ascii_cell(Some(0x41)).0, 'A');
        assert_eq!(ascii_cell(Some(0x20)).0, ' ');
        assert_eq!(ascii_cell(Some(0x7E)).0, '~');
        // Non-printable published bytes render as a dot, still tinted.
        assert_eq!(ascii_cell(Some(0x00)).0, '.');
        assert_eq!(ascii_cell(Some(0xFF)).0, '.');
        assert_eq!(ascii_cell(Some(0x00)).1, Some(byte_color(0x00)));
        // A gap (unpublished running byte) blanks its ASCII column, untinted.
        assert_eq!(ascii_cell(None), (' ', None));
    }

    #[test]
    fn byte_hue_spans_the_range_monotonically() {
        // Endpoints land at the intended bounds — no wheel wraparound.
        assert_eq!(byte_hue(0x00), 0.0);
        assert_eq!(byte_hue(0xFF), HUE_SPAN);
        // Strictly increasing across the whole byte range.
        for value in 0..0xFFu8 {
            assert!(byte_hue(value) < byte_hue(value + 1));
        }
    }

    #[test]
    fn byte_color_endpoints_are_distinct() {
        // 0x00 and 0xFF must not collide on the same hue.
        assert_ne!(byte_hue(0x00), byte_hue(0xFF));
        assert_ne!(byte_color(0x00), byte_color(0xFF));
    }

    #[test]
    fn row_spans_tint_hex_and_ascii_with_the_byte_colour() {
        let spans = row_spans(0xC100, &cells(&[0x41, 0x7F]));
        // 'A' (printable): its hex pair and glyph share the byte's colour.
        assert!(spans.contains(&("41".to_owned(), Some(byte_color(0x41)))));
        assert!(spans.contains(&("A".to_owned(), Some(byte_color(0x41)))));
        // 0x7F (non-printable): hex pair and the fallback dot share the colour.
        assert!(spans.contains(&("7F".to_owned(), Some(byte_color(0x7F)))));
        assert!(spans.contains(&(".".to_owned(), Some(byte_color(0x7F)))));
    }

    #[test]
    fn row_text_full_line() {
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(
            row_text(0xC100, &cells(&bytes)),
            "$C100  00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F  |................|"
        );
    }

    #[test]
    fn row_text_short_line_pads_hex_columns() {
        let row = row_text(0xFF80, &cells(&[0x48, 0x49]));
        // Two bytes rendered, the remaining fourteen columns blanked, ASCII
        // gutter only as wide as the bytes present.
        assert_eq!(
            row,
            "$FF80  48 49                                            |HI|"
        );
    }

    #[test]
    fn row_unpublished_cell_reads_as_dashes() {
        let row = row_text(0xC100, &[Some(0x10), None, Some(0x30)]);
        assert!(row.starts_with("$C100  10 -- 30"));
        assert!(row.ends_with("|. 0|"));
        // The `--` placeholder and its blank ASCII column stay muted, untinted.
        let spans = row_spans(0xC100, &[Some(0x10), None, Some(0x30)]);
        assert!(spans.contains(&("--".to_owned(), Some(palette::MUTED))));
        assert!(spans.contains(&(" ".to_owned(), None)));
    }

    #[test]
    fn window_range_clamps_to_a_short_region() {
        // A 0xA0-byte region (OAM) is smaller than one window: never scrolls.
        let (offset, base, visible) = window_range(0xFE00, 0xA0, 0);
        assert_eq!((offset, base, visible), (0, 0xFE00, 0xA0));
        // Any requested offset clamps back to the single page.
        let (offset, base, visible) = window_range(0xFE00, 0xA0, 0x400);
        assert_eq!((offset, base, visible), (0, 0xFE00, 0xA0));
    }

    #[test]
    fn window_range_snaps_and_clamps_at_region_end() {
        // 0x4000-byte region: mid-region scroll snaps to a row boundary.
        let (offset, base, _) = window_range(0x0000, 0x4000, 0x1234);
        assert_eq!(offset, 0x1230);
        assert_eq!(base, 0x1230);
        // Past the end clamps to the last full page.
        let ceiling = max_offset(0x4000);
        let (offset, _, visible) = window_range(0x0000, 0x4000, 0xFFFF);
        assert_eq!(offset, ceiling);
        assert_eq!(visible, VISIBLE_BYTES);
    }

    #[test]
    fn max_offset_zero_when_region_fits_in_a_window() {
        assert_eq!(max_offset(VISIBLE_BYTES), 0);
        assert_eq!(max_offset(VISIBLE_BYTES - 1), 0);
        assert_eq!(max_offset(VISIBLE_BYTES + 1), BYTES_PER_ROW);
    }

    #[test]
    fn interest_matches_the_visible_window() {
        let regions = [
            region("rom", 0x0000, 0x4000),
            region("wram", 0xC000, 0x2000),
        ];
        // Region 1 at offset 0x100 → base 0xC100, one full screen.
        let interest = interest_for(
            &regions,
            MemorySelection {
                region: 1,
                offset: 0x105,
            },
        )
        .unwrap();
        assert_eq!(
            interest,
            MemoryInterest {
                start: 0xC100,
                len: VISIBLE_BYTES,
            }
        );
        // No regions → no interest.
        assert_eq!(
            interest_for(
                &[],
                MemorySelection {
                    region: 0,
                    offset: 0
                }
            ),
            None
        );
    }

    #[test]
    fn interest_short_region_shrinks_to_fit() {
        let regions = [region("oam", 0xFE00, 0xA0)];
        let interest = interest_for(
            &regions,
            MemorySelection {
                region: 0,
                offset: 0,
            },
        )
        .unwrap();
        assert_eq!(interest.len, 0xA0);
    }

    #[test]
    fn resolve_jump_finds_region_and_snaps_offset() {
        let regions = [
            region("rom", 0x0000, 0x4000),
            region("wram", 0xC000, 0x2000),
        ];
        // An address inside wram resolves to that region, row-snapped.
        assert_eq!(resolve_jump(&regions, "C123"), Some((1, 0x120)));
        // Leading $ and 0x are accepted.
        assert_eq!(resolve_jump(&regions, "$C000"), Some((1, 0x000)));
        assert_eq!(resolve_jump(&regions, "0x0040"), Some((0, 0x40)));
        // An address in no region, and unparseable input, both reject.
        assert_eq!(resolve_jump(&regions, "E000"), None);
        assert_eq!(resolve_jump(&regions, "wram"), None);
        assert_eq!(resolve_jump(&regions, ""), None);
    }

    #[test]
    fn resolve_jump_clamps_to_the_last_page() {
        let regions = [region("wram", 0xC000, 0x2000)];
        // The final byte of a large region clamps to its last full page.
        let ceiling = max_offset(0x2000);
        assert_eq!(resolve_jump(&regions, "DFFF"), Some((0, ceiling)));
    }

    #[test]
    fn windowed_source_reads_covered_bytes_and_misses_the_rest() {
        let window = MemoryWindow {
            base: 0xC100,
            bytes: vec![0x10, 0x20, 0x30, 0x40],
        };
        // Covered addresses read through.
        assert_eq!(window.read(0xC100), Some(0x10));
        assert_eq!(window.read(0xC103), Some(0x40));
        // Just outside the published span → an unpublished cell.
        assert_eq!(window.read(0xC104), None);
        assert_eq!(window.read(0xC0FF), None);
    }

    #[test]
    fn interests_union_across_two_panes() {
        // Two memory panes on different regions produce two interest spans.
        let regions = [
            region("wram", 0xC000, 0x2000),
            region("sram", 0xA000, 0x2000),
        ];
        let selections = [
            MemorySelection {
                region: 0,
                offset: 0,
            },
            MemorySelection {
                region: 1,
                offset: 0,
            },
        ];
        let interests = interests_for(&regions, &selections);
        assert_eq!(interests.len(), 2);
        assert_eq!(interests[0].start, 0xC000);
        assert_eq!(interests[1].start, 0xA000);
    }

    #[test]
    fn each_pane_matches_its_own_window_by_base() {
        // The acceptance scenario: one pane on wram, one on the linear sram
        // region; the emu thread peeks one window per interest and each pane
        // resolves its own by base.
        let regions = vec![
            region("wram", 0xC000, 0x2000),
            region("sram", 0xA000, 0x2000),
        ];
        let mut wram_pane = MemoryPane::new();
        wram_pane.selection.region = 0;
        let mut sram_pane = MemoryPane::new();
        sram_pane.selection.region = 1;

        let windows: Vec<MemoryWindow> =
            interests_for(&regions, &[wram_pane.selection, sram_pane.selection])
                .iter()
                .map(|interest| MemoryWindow {
                    base: interest.start,
                    bytes: vec![0xAB; interest.len as usize],
                })
                .collect();

        let wram_base = wram_pane.base(&regions);
        let sram_base = sram_pane.base(&regions);
        assert_eq!(wram_base, 0xC000);
        assert_eq!(sram_base, 0xA000);
        assert_ne!(wram_base, sram_base);
        // Each pane finds a matching published window.
        assert!(windows.iter().any(|window| window.base == wram_base));
        assert!(windows.iter().any(|window| window.base == sram_base));
    }
}
