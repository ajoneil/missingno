/// The propagation state of a value moving through a DFF cell.
///
/// On hardware, the CPU write pulse sets D on the master latch.
/// What happens next depends on the cell type:
/// - DFF8: edge-triggered capture. The old output persists until the
///   capture tick fires (one dot after write), then atomically
///   transitions to the new value.
/// - DFF9: the value latches atomically, but internal signal routing
///   may delay when the new value appears on the output pin.
pub(super) enum LatchState {
    /// DFF8 pending capture: the master latch holds a new value, but
    /// the slave still outputs the old value. On the first tick after
    /// write, the pending state becomes ready; on the second tick,
    /// the slave captures and the output transitions atomically.
    Pending { final_value: u8, ready: bool },
    /// DFF9 propagation: the old value persists on the output until the
    /// next tick. The tick runs after the pipeline, so the pipeline reads
    /// the pre-write value (reg_old) and the tick resolves afterward.
    Propagating { final_value: u8 },
}

/// A DFF register cell that holds its output value and any pending latch.
///
/// On hardware, each register is a physical DFF cell whose output feeds
/// the pixel pipeline. The CPU writes to the cell's input; the latch
/// state tracks how the new value propagates to the output.
///
/// Outside Mode 3, writes go directly to `output` (no latch state).
/// During Mode 3, the write behavior depends on the cell type.
pub struct DffLatch {
    pub(super) output: u8,
    pub(super) state: Option<LatchState>,
}

impl DffLatch {
    pub(super) fn new(initial: u8) -> Self {
        Self {
            output: initial,
            state: None,
        }
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    /// Advance the latch state by one dot. Returns true if the latch
    /// resolved (final value applied) on this tick.
    pub(super) fn tick(&mut self) -> bool {
        match self.state {
            Some(LatchState::Pending { final_value, ready }) => {
                if !ready {
                    // First tick after write: become ready, output unchanged (still old).
                    self.state = Some(LatchState::Pending {
                        final_value,
                        ready: true,
                    });
                    false
                } else {
                    // Second tick: slave captures, resolve to final value.
                    self.output = final_value;
                    self.state = None;
                    true
                }
            }
            Some(LatchState::Propagating { final_value }) => {
                self.output = final_value;
                self.state = None;
                true
            }
            None => false,
        }
    }

    /// DFF8 write during Mode 3. The old output persists; the new value
    /// is held as pending until the capture tick fires.
    pub(super) fn write_dff8(&mut self, new_value: u8) {
        self.state = Some(LatchState::Pending {
            final_value: new_value,
            ready: false,
        });
    }

    /// DFF9 write during Mode 3. The old value persists on the output
    /// until the next tick (which runs after the pipeline).
    pub(super) fn write_propagating(&mut self, new_value: u8) {
        self.state = Some(LatchState::Propagating {
            final_value: new_value,
        });
    }

    /// Direct write — sets the output immediately and clears any
    /// pending latch state.
    pub(super) fn write_immediate(&mut self, new_value: u8) {
        self.output = new_value;
        self.state = None;
    }

    /// Clear pending latch state without applying the final value.
    pub(super) fn clear(&mut self) {
        self.state = None;
    }
}
