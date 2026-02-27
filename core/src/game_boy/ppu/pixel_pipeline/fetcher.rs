// --- Background fetcher ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FetcherStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    /// The fetcher has completed all three VRAM reads and is frozen,
    /// waiting for the SEKO-triggered reload (fine_count == 7).
    Idle,
}

/// Mode 3 starts with one BG tile fetch before any pixels shift out.
/// On hardware, AVAP fires at Mode 3 entry and the fetcher begins
/// immediately. After the first tile fetch completes, the LYRY
/// combinational signal and NYKA→PORY→POKY DFF cascade propagate the
/// "fetch done" signal across alternating clock phases before enabling
/// the pixel clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartupFetch {
    /// First tile fetch in progress. The fetcher runs on DELTA_EVEN only
    /// (LEBO clock). When the fetcher fills the BG shifter, LYRY fires
    /// combinationally → transitions to LyryFired.
    FirstTile,

    /// LYRY_BFETCH_DONEn has fired (combinational — the fetcher filled
    /// the shifter this DELTA_EVEN). NYKA will capture it on the *next*
    /// DELTA_EVEN.
    LyryFired,

    /// NYKA_FETCH_DONEp_evn has captured LYRY. PORY will capture NYKA
    /// on the next DELTA_ODD.
    NykaFired,

    /// PORY_FETCH_DONEp_odd has captured NYKA. POKY will latch on the
    /// next DELTA_EVEN, enabling the pixel clock (startup_fetch → None).
    PoryFired,
}

/// Which half of LEBO's 2-dot clock cycle the fetcher is in.
/// The fetcher (and OAM scanner) are clocked at half the dot rate.
/// T1 is the first dot (LEBO low → high edge); T2 is the second
/// (LEBO high → low edge, when the actual work fires).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FetcherTick {
    T1,
    T2,
}

pub(super) struct TileFetcher {
    pub(super) step: FetcherStep,
    /// Which half of the 2-dot fetcher clock cycle we're in.
    pub(super) tick: FetcherTick,
    /// Window tile X counter (hardware's win_x.map). Increments per
    /// window tile fetched. Reset to 0 on window trigger.
    pub(super) window_tile_x: u8,
    /// Cached tile index from GetTile step.
    pub(super) tile_index: u8,
    /// Cached low byte of tile row from GetTileDataLow step.
    pub(super) tile_data_low: u8,
    /// Cached high byte of tile row from GetTileDataHigh step.
    pub(super) tile_data_high: u8,
    /// Whether we're fetching from the window tilemap.
    pub(super) fetching_window: bool,
}

impl TileFetcher {
    pub(super) fn new() -> Self {
        Self {
            step: FetcherStep::GetTile,
            tick: FetcherTick::T1,
            window_tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
        }
    }
}
