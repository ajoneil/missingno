/// The propagation state of a value moving through a DFF cell.
///
/// On hardware, the CPU write pulse sets D on the master latch.
/// What happens next depends on the cell type:
/// - DFF8: master-slave transparency produces `old | new` briefly,
///   then the slave settles to the final value on the next dot.
/// - DFF9: the value latches atomically, but internal signal routing
///   may delay when the new value appears on the output pin.
pub(super) enum LatchState {
    /// DFF8 transitional: output is `old | new` while the master latch
    /// is transparent. A fresh transitional survives one tick (modeling
    /// persistence through the write-phase clock edge); the next tick
    /// resolves to the final value.
    Transitional { final_value: u8, fresh: bool },
    /// DFF9 propagation: the old value persists on the output for two
    /// dots while the new value routes through internal wiring. First
    /// tick marks as stale; second tick applies the final value. This
    /// matches hardware's G→H latch boundary at dot 3 falling.
    Propagating { final_value: u8, fresh: bool },
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
            Some(LatchState::Transitional { final_value, fresh }) => {
                if fresh {
                    // First tick after write: mark as stale, keep old|new output.
                    self.state = Some(LatchState::Transitional {
                        final_value,
                        fresh: false,
                    });
                    false
                } else {
                    // Second tick: slave captures, resolve to final value.
                    self.output = final_value;
                    self.state = None;
                    true
                }
            }
            Some(LatchState::Propagating { final_value, fresh }) => {
                if fresh {
                    self.state = Some(LatchState::Propagating {
                        final_value,
                        fresh: false,
                    });
                    false
                } else {
                    self.output = final_value;
                    self.state = None;
                    true
                }
            }
            None => false,
        }
    }

    /// DFF8 write during Mode 3. Sets the transitional `old | new`
    /// output and begins the transitional phase.
    pub(super) fn write_dff8(&mut self, new_value: u8) {
        self.output = self.output | new_value;
        self.state = Some(LatchState::Transitional {
            final_value: new_value,
            fresh: true,
        });
    }

    /// DFF9 write during Mode 3. The old value persists on the output
    /// for two dots while the new value propagates through internal wiring.
    pub(super) fn write_propagating(&mut self, new_value: u8) {
        self.state = Some(LatchState::Propagating {
            final_value: new_value,
            fresh: true,
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
