//! Debug proof of the deferral: a clone of the video-control state ticked
//! through the real dot fall on every dot the span skips, held against the live
//! state at each one and compared field for field the moment the closed form
//! materialises. A wrong closed form or a constancy claim that does not hold
//! fails at the exact dot rather than as a frame difference.

use super::stat_interrupt::StatShadow;
use super::video_control::VideoControl;

pub(in crate::ppu) struct SpanShadow {
    video: VideoControl,
}

impl SpanShadow {
    pub(in crate::ppu) fn seed(video: &VideoControl) -> Self {
        Self {
            video: video.clone(),
        }
    }

    /// Run the dot fall the span is skipping. Only LX and the divider phase may
    /// move; the live state must still read as the shadow does everywhere else.
    pub(in crate::ppu) fn step(&mut self, live: &VideoControl, stat: &impl StatShadow) {
        let advance = self.video.advance_dot(stat);
        assert!(
            !advance.scanline_boundary,
            "a slept dot crossed the line end"
        );
        assert!(!advance.vblank_rose, "a slept dot raised POPU");
        assert!(!self.video.line_end_active(), "a slept dot raised RUTU");
        assert_eq!(self.video.ly(), live.ly(), "a slept dot moved LY");
        assert_eq!(self.video.vblank(), live.vblank(), "a slept dot moved POPU");
        assert_eq!(
            self.video.stat.ly_eq_lyc(),
            live.stat.ly_eq_lyc(),
            "a slept dot moved ROPO"
        );
    }

    pub(in crate::ppu) fn compare(self, live: &VideoControl) {
        assert_eq!(
            *live, self.video,
            "the deferred closed form diverged from the dot-by-dot chain"
        );
    }
}
