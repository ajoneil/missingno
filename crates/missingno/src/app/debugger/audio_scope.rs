//! The debugger's audio scope: one stacked trace per sound channel, plotting
//! the DAC input codes each channel captured this window. Codes are normalized
//! by the channel's bit depth (a 4-bit channel spans 0..15 full-scale) and
//! drawn as a per-pixel-column min/max envelope so peaks survive downsampling
//! to the pane width. The waveforms cross the seam family-agnostically, so this
//! one pane serves every family whose core captures them.

use iced::{
    Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse,
    widget::{
        Column, canvas,
        canvas::{Frame, Geometry},
        container, row, text,
    },
};

use crate::app::{
    Message,
    debugger::panes::{self, pane, title_bar},
    ui::{
        palette,
        sizes::{s, xs},
    },
};
use missingno_core::waveform::ChannelWave;

/// Per-channel trace colours, applied in registration order and wrapped if a
/// family ever exceeds them.
const CHANNEL_COLORS: [Color; 4] = [
    palette::PURPLE,
    palette::TEAL,
    palette::PEACH,
    palette::YELLOW,
];

/// Fixed width of each channel's label gutter.
const LABEL_WIDTH: f32 = 52.0;

pub struct AudioScopePane;

impl AudioScopePane {
    pub fn new() -> Self {
        Self
    }
}

impl panes::Pane for AudioScopePane {
    fn kind(&self) -> panes::DebuggerPane {
        panes::DebuggerPane::Audio
    }

    fn view<'a>(
        &'a self,
        ctx: Option<&panes::PaneContext<'_>>,
        id: iced::widget::pane_grid::Pane,
    ) -> iced::widget::pane_grid::Content<'a, Message> {
        match ctx
            .and_then(|ctx| ctx.waves)
            .filter(|waves| !waves.is_empty())
        {
            Some(waves) => scopes(waves, id),
            None => capture_off(id),
        }
    }
}

/// The stacked per-channel scopes.
fn scopes<'a>(
    waves: &[ChannelWave],
    close: iced::widget::pane_grid::Pane,
) -> iced::widget::pane_grid::Content<'a, Message> {
    let rows = waves
        .iter()
        .enumerate()
        .map(|(index, wave)| channel_row(wave, CHANNEL_COLORS[index % CHANNEL_COLORS.len()]));
    let body = Column::from_iter(rows).spacing(s()).padding(s());
    pane(title_bar("Audio", close), body.into())
}

/// The hint shown when no waveform window is available — capture disabled, or
/// the core has not published one yet.
fn capture_off<'a>(
    close: iced::widget::pane_grid::Pane,
) -> iced::widget::pane_grid::Content<'a, Message> {
    pane(
        title_bar("Audio", close),
        container(text("Waveform capture off").color(palette::MUTED))
            .center(Length::Fill)
            .into(),
    )
}

/// One channel's row: a label gutter (name over an activity pip) beside the
/// trace that fills the rest of the width.
fn channel_row<'a>(wave: &ChannelWave, color: Color) -> Element<'a, Message> {
    let pip = text(if wave.active { "\u{25CF}" } else { "\u{25CB}" })
        .size(11.0)
        .color(if wave.active {
            palette::GREEN
        } else {
            palette::SURFACE2
        });
    let gutter = container(
        Column::from_iter([text(wave.label).size(12.0).into(), pip.into()]).spacing(xs()),
    )
    .width(Length::Fixed(LABEL_WIDTH));

    let scope = canvas(ChannelScope {
        levels: wave.levels.clone(),
        max_code: max_code(wave.depth_bits),
        color,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    container(row![gutter, scope].spacing(s()))
        .height(Length::FillPortion(1))
        .into()
}

/// The full-scale code for a channel of `depth_bits` resolution.
fn max_code(depth_bits: u8) -> f32 {
    ((1u32 << depth_bits.min(31)) - 1).max(1) as f32
}

/// A single channel's canvas trace.
struct ChannelScope {
    levels: Vec<u8>,
    max_code: f32,
    color: Color,
}

impl canvas::Program<Message> for ChannelScope {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);

        // Subtle baseline across the vertical midpoint.
        frame.fill_rectangle(
            Point::new(0.0, h / 2.0 - 0.5),
            Size::new(w, 1.0),
            Color {
                a: 0.12,
                ..palette::MUTED
            },
        );

        let cols = w.floor() as usize;
        // Code 0 sits at the bottom, full-scale at the top, with a hair of pad.
        let pad = 1.0;
        let usable = (h - 2.0 * pad).max(1.0);
        let y_of = |code: f32| pad + (1.0 - code / self.max_code) * usable;

        for (col, (min, max)) in column_minmax(&self.levels, cols).into_iter().enumerate() {
            let top = y_of(max as f32);
            let bottom = y_of(min as f32);
            let bar = (bottom - top).max(1.0);
            frame.fill_rectangle(Point::new(col as f32, top), Size::new(1.0, bar), self.color);
        }

        vec![frame.into_geometry()]
    }
}

/// Reduce `levels` to `cols` columns, each carrying the (min, max) of the
/// samples that fall in it — the standard scope treatment, so a peak in a
/// column that averages low still reaches full height. Columns beyond the
/// sample count repeat the nearest sample (an honest upsample of a short
/// window), and an empty window or zero width yields nothing.
fn column_minmax(levels: &[u8], cols: usize) -> Vec<(u8, u8)> {
    let n = levels.len();
    if n == 0 || cols == 0 {
        return Vec::new();
    }
    (0..cols)
        .filter_map(|col| {
            let start = col * n / cols;
            let end = ((col + 1) * n / cols).max(start + 1).min(n);
            if start >= n {
                return None;
            }
            let slice = &levels[start..end];
            Some((*slice.iter().min().unwrap(), *slice.iter().max().unwrap()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::column_minmax;

    #[test]
    fn column_minmax_preserves_peaks() {
        // Eight samples reduced to four columns: each column spans two samples,
        // keeping both the trough and the crest.
        let levels = [0u8, 7, 1, 6, 2, 5, 3, 4];
        assert_eq!(
            column_minmax(&levels, 4),
            vec![(0, 7), (1, 6), (2, 5), (3, 4)]
        );

        // A lone spike survives a heavy reduction rather than aliasing away.
        let mut spiky = vec![0u8; 100];
        spiky[50] = 15;
        let reduced = column_minmax(&spiky, 10);
        assert_eq!(reduced.len(), 10);
        assert!(reduced.iter().any(|&(_, max)| max == 15));
    }

    #[test]
    fn column_minmax_upsamples_a_short_window() {
        let cols = column_minmax(&[3, 9], 6);
        assert_eq!(cols.len(), 6);
        assert_eq!(cols.first(), Some(&(3, 3)));
        assert_eq!(cols.last(), Some(&(9, 9)));
    }

    #[test]
    fn column_minmax_empty_inputs() {
        assert!(column_minmax(&[], 10).is_empty());
        assert!(column_minmax(&[1, 2, 3], 0).is_empty());
    }
}
