//! The broadcast standard and the raster geometry it fixes.

/// The crystal is the board's master grid: 3 XTAL periods per CPU
/// T-state, 2 per dot, 684 per line.
pub const XTALS_PER_TSTATE: u32 = 3;
pub(crate) const XTALS_PER_LINE: u32 = 684;

/// The display area: the window the planes are resolved in.
pub const ACTIVE_WIDTH: u16 = 256;
pub const ACTIVE_LINES: u16 = 192;
/// The backdrop border around it, side dots per the Data Manual.
pub const LEFT_BORDER: u16 = 13;
pub const RIGHT_BORDER: u16 = 15;
pub const VISIBLE_WIDTH: u16 = LEFT_BORDER + ACTIVE_WIDTH + RIGHT_BORDER;

/// The broadcast standard the part is cut for: TMS9918A (NTSC) or
/// TMS9929A (PAL).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standard {
    Ntsc,
    Pal,
}

impl Standard {
    pub fn lines_per_frame(self) -> u16 {
        match self {
            Standard::Ntsc => 262,
            Standard::Pal => 313,
        }
    }

    /// Border lines above the display area; the 9929A's split is derived,
    /// no TI document giving its 313-line breakdown.
    pub fn top_border(self) -> u16 {
        match self {
            Standard::Ntsc => 27,
            Standard::Pal => 51,
        }
    }

    pub fn bottom_border(self) -> u16 {
        match self {
            Standard::Ntsc => 24,
            Standard::Pal => 51,
        }
    }

    pub fn visible_lines(self) -> u16 {
        self.top_border() + ACTIVE_LINES + self.bottom_border()
    }
}
