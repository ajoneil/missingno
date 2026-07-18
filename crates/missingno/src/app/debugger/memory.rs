//! A generic memory viewer pane, registered for every family. While paused it
//! reads live through the seam's `memory_regions()` + `peek()`, showing a
//! region picker and a scrollable hex/ASCII grid. While the core runs on the
//! emulation thread it can only show what the per-vblank snapshot captured — a
//! bounded, PC-anchored window — with a note that full memory needs a pause.

use iced::widget::{Column, button, column, container, pane_grid, pick_list, row, text};
use iced::{Element, Length};

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

/// The eagerly-copied data the pane renders from, built afresh each frame so
/// the pane never borrows the core. `regions` is empty and `running` is true
/// when the bytes come from a running snapshot window.
#[derive(Clone, Copy)]
pub struct MemoryPaneData<'b> {
    pub regions: &'b [MemoryRegion],
    pub selected: usize,
    pub base: u32,
    pub bytes: &'b [u8],
    pub running: bool,
}

/// An owned readout the context builder holds so the pane can borrow its bytes.
pub struct MemoryReadout {
    pub regions: &'static [MemoryRegion],
    pub selected: usize,
    pub base: u32,
    pub bytes: Vec<u8>,
}

impl<'b> MemoryPaneData<'b> {
    /// The paused view over a live peek readout.
    pub fn paused(readout: &'b MemoryReadout) -> Self {
        Self {
            regions: readout.regions,
            selected: readout.selected,
            base: readout.base,
            bytes: &readout.bytes,
            running: false,
        }
    }

    /// The running view over the snapshot's PC-anchored window.
    pub fn running(window: &'b MemoryWindow) -> Self {
        Self {
            regions: &[],
            selected: 0,
            base: window.base,
            bytes: &window.bytes,
            running: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SelectRegion(usize),
    SetOffset(u32),
}

impl From<Message> for app::Message {
    fn from(val: Message) -> Self {
        panes::Message::Pane(PaneMessage::Memory(val)).into()
    }
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

/// Copy the bytes the paused memory pane should show for `selection`, reading
/// side-effect-free through the seam. Bounded to one screen's worth.
pub fn build_readout(core: &dyn SystemDebugger, selection: MemorySelection) -> MemoryReadout {
    let regions = core.memory_regions();
    if regions.is_empty() {
        return MemoryReadout {
            regions,
            selected: 0,
            base: 0,
            bytes: Vec::new(),
        };
    }
    let selected = selection.region.min(regions.len() - 1);
    let region = regions[selected];
    let (_, base, visible) = window_range(region.start, region.len, selection.offset);
    let bytes = (0..visible).map(|i| core.peek(base + i)).collect();
    MemoryReadout {
        regions,
        selected,
        base,
        bytes,
    }
}

fn ascii_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// One grid line: `$ADDR  XX XX .. XX  |ascii|`, with short final rows padded so
/// the ASCII gutter stays aligned in the monospace font.
fn hex_row(address: u32, bytes: &[u8]) -> String {
    let mut hex = String::new();
    for i in 0..BYTES_PER_ROW as usize {
        if i > 0 {
            hex.push(' ');
        }
        match bytes.get(i) {
            Some(byte) => hex.push_str(&format!("{byte:02X}")),
            None => hex.push_str("  "),
        }
    }
    format!("${address:04X}  {hex}  |{}|", ascii_dump(bytes))
}

pub struct MemoryPane {
    selection: MemorySelection,
}

impl MemoryPane {
    pub fn new() -> Self {
        Self {
            selection: MemorySelection {
                region: 0,
                offset: 0,
            },
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SelectRegion(region) => {
                self.selection.region = region;
                self.selection.offset = 0;
            }
            Message::SetOffset(offset) => self.selection.offset = offset,
        }
    }

    fn grid(base: u32, bytes: &[u8]) -> Element<'static, app::Message> {
        let rows = bytes
            .chunks(BYTES_PER_ROW as usize)
            .enumerate()
            .map(|(i, row)| {
                text(hex_row(base + i as u32 * BYTES_PER_ROW, row))
                    .font(fonts::monospace())
                    .size(13.0)
                    .color(palette::TEXT)
                    .into()
            });
        Column::from_iter(rows).into()
    }

    fn paused_view(&self, data: &MemoryPaneData<'_>) -> Element<'static, app::Message> {
        let region = data.regions[data.selected];
        let choices: Vec<RegionChoice> = data
            .regions
            .iter()
            .enumerate()
            .map(|(index, region)| RegionChoice::new(index, region))
            .collect();
        let selected = RegionChoice::new(data.selected, &region);
        let picker = pick_list(choices, Some(selected), |choice| {
            Message::SelectRegion(choice.index).into()
        })
        .font(fonts::monospace())
        .text_size(13.0);

        let offset = self.selection.offset / BYTES_PER_ROW * BYTES_PER_ROW;
        let ceiling = max_offset(region.len);
        let up = scroll_button(
            "\u{2191}",
            offset > 0,
            Message::SetOffset(offset.saturating_sub(BYTES_PER_ROW)),
        );
        let down = scroll_button(
            "\u{2193}",
            offset < ceiling,
            Message::SetOffset((offset + BYTES_PER_ROW).min(ceiling)),
        );
        let page_up = scroll_button(
            "\u{21C8}",
            offset > 0,
            Message::SetOffset(offset.saturating_sub(VISIBLE_BYTES)),
        );
        let page_down = scroll_button(
            "\u{21CA}",
            offset < ceiling,
            Message::SetOffset((offset + VISIBLE_BYTES).min(ceiling)),
        );
        let controls = row![picker, page_up, up, down, page_down]
            .spacing(s())
            .align_y(iced::alignment::Vertical::Center);

        column![controls, Self::grid(data.base, data.bytes)]
            .spacing(s())
            .padding(s())
            .into()
    }

    fn running_view(data: &MemoryPaneData<'_>) -> Element<'static, app::Message> {
        let hint = text("Pause to browse full memory")
            .font(fonts::monospace())
            .size(11.0)
            .color(palette::MUTED);
        column![hint, Self::grid(data.base, data.bytes)]
            .spacing(s())
            .padding(s())
            .into()
    }
}

fn scroll_button(
    glyph: &'static str,
    enabled: bool,
    message: Message,
) -> Element<'static, app::Message> {
    let label = text(glyph).font(fonts::monospace()).size(13.0);
    let mut btn = button(label).style(button::text);
    if enabled {
        btn = btn.on_press(message.into());
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

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(data) = ctx.and_then(|ctx| ctx.memory) else {
            return panes::running_placeholder("Memory");
        };
        let body = if data.running {
            MemoryPane::running_view(&data)
        } else if data.regions.is_empty() {
            container(
                text("No memory map")
                    .font(fonts::monospace())
                    .color(palette::MUTED),
            )
            .center(Length::Fill)
            .into()
        } else {
            self.paused_view(&data)
        };
        panes::pane(panes::title_bar("Memory"), body)
    }

    fn on_message(&mut self, message: &PaneMessage) {
        if let PaneMessage::Memory(message) = message {
            self.update(*message);
        }
    }

    fn memory_selection(&self) -> Option<MemorySelection> {
        Some(self.selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_dump_maps_printable_and_dots_the_rest() {
        assert_eq!(ascii_dump(&[0x41, 0x42, 0x43]), "ABC");
        assert_eq!(ascii_dump(&[0x00, 0x1F, 0x7F, 0x80, 0xFF]), ".....");
        assert_eq!(ascii_dump(&[0x20, 0x7E]), " ~");
    }

    #[test]
    fn hex_row_full_line() {
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(
            hex_row(0xC100, &bytes),
            "$C100  00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F  |................|"
        );
    }

    #[test]
    fn hex_row_short_line_pads_hex_columns() {
        let row = hex_row(0xFF80, &[0x48, 0x49]);
        // Two bytes rendered, the remaining fourteen columns blanked, ASCII
        // gutter only as wide as the bytes present.
        assert_eq!(
            row,
            "$FF80  48 49                                            |HI|"
        );
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
}
