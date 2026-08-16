//! The WSYNC RDY latch.

/// SHB's latched reset absorbs a WSYNC set through the wrap's first CPU cycle.
const WSYNC_RESET_HOLD_CLOCKS: u8 = 3;

/// The WSYNC RDY latch. A strobe drops RDY to park the CPU until the line
/// wrap releases it; SHB's latched reset outlasts that wrap, and while it
/// holds a WSYNC set is overridden and never reaches RDY.
pub(super) struct RdyLatch {
    ready: bool,
    reset_hold: u8,
}

impl RdyLatch {
    pub(super) fn new() -> Self {
        RdyLatch {
            ready: true,
            reset_hold: 0,
        }
    }

    pub(super) fn ready(&self) -> bool {
        self.ready
    }

    pub(super) fn step(&mut self) {
        self.reset_hold = self.reset_hold.saturating_sub(1);
    }

    pub(super) fn strobe(&mut self) {
        if self.reset_hold == 0 {
            self.ready = false;
        }
    }

    pub(super) fn release(&mut self) {
        self.ready = true;
        self.reset_hold = WSYNC_RESET_HOLD_CLOCKS;
    }

    pub(super) fn capture(&self) -> (bool, u8) {
        (self.ready, self.reset_hold)
    }

    pub(super) fn restore(&mut self, ready: bool, reset_hold: u8) {
        self.ready = ready;
        self.reset_hold = reset_hold;
    }
}
