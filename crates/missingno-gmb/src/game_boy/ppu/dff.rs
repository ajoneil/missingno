/// A DFF register cell that holds its output value and any pending latch.
///
/// On hardware, each register is a physical DFF cell whose output feeds
/// the pixel pipeline. The CPU writes to the cell's input; the pending
/// value resolves on the next tick of the appropriate clock edge.
///
/// Outside Mode 3, writes go directly to `output` (no pending state).
/// During Mode 3, the old output persists until the capture tick fires.
pub struct DffLatch {
    pub(super) output: u8,
    /// A value waiting to be captured on the next tick. Both DFF8
    /// (palette registers, captured on falling/phase H) and DFF9
    /// (viewport/control registers, captured on falling after pipeline)
    /// use the same mechanism: write sets pending, next tick resolves.
    pub(super) pending: Option<u8>,
}

impl DffLatch {
    pub(super) fn new(initial: u8) -> Self {
        Self {
            output: initial,
            pending: None,
        }
    }

    pub fn output(&self) -> u8 {
        self.output
    }

    /// Advance the latch: if a value is pending, capture it to output.
    /// Returns true if the latch resolved on this tick.
    pub(super) fn tick(&mut self) -> bool {
        if let Some(value) = self.pending.take() {
            self.output = value;
            true
        } else {
            false
        }
    }

    /// Write during Mode 3. The old output persists until the next
    /// falling-phase tick resolves the pending value.
    pub(super) fn write(&mut self, new_value: u8) {
        self.pending = Some(new_value);
    }

    /// Direct write — sets the output immediately and clears any
    /// pending state.
    pub(super) fn write_immediate(&mut self, new_value: u8) {
        self.output = new_value;
        self.pending = None;
    }

    /// Clear pending state without applying the value.
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }
}
