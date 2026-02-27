// --- Sprite fetch ---

use super::fetcher::FetcherTick;
use super::oam_scan::SpriteStoreEntry;

/// The two phases of a sprite fetch on real hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpriteFetchPhase {
    /// The BG fetcher continues advancing through its normal steps.
    /// The wait ends when the fetcher has completed GetTileDataHigh
    /// (reached Load) AND the BG shifter is non-empty — both conditions
    /// must be true simultaneously. The variable sprite penalty (0-5
    /// dots) emerges from how many fetcher steps this phase consumes.
    WaitingForFetcher,
    /// The BG fetcher is frozen at its current position. Sprite tile
    /// data is read through the SpriteStep state machine (6 dots total).
    FetchingData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpriteStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
}

pub(super) struct SpriteFetch {
    /// The sprite store entry that triggered this fetch.
    pub(super) entry: SpriteStoreEntry,
    pub(super) phase: SpriteFetchPhase,
    pub(super) step: SpriteStep,
    pub(super) tick: FetcherTick,
    pub(super) tile_data_low: u8,
    pub(super) tile_data_high: u8,
}

/// Sprite fetch lifecycle. On hardware, FEPO (sprite X match) freezes
/// the pixel clock, the fetch runs, then the pixel clock resumes
/// normally on the next dot (state_old.FEPO=0).
pub(super) enum SpriteState {
    /// No sprite activity. Pixel clock runs normally.
    Idle,
    /// Sprite fetch in progress (wait + data phases).
    Fetching(SpriteFetch),
}
