/// A generic D flip-flop latch modelling the capture semantics of a
/// hardware DFF cell.
///
/// Writes set the `pending` D input; `tick()` captures D → Q on the next
/// clock edge; `output()` reads Q — the most recently captured value.
/// Readers observe pre-edge Q until `tick` fires, matching hardware where
/// combinational consumers of a DFF's output see the old value right up
/// to the clocking edge.
///
/// The PPU has a u8-specific `DffLatch` (`ppu::dff`) with the same
/// semantics; a future refactor can collapse it into `Dff<u8>`.
pub struct Dff<T> {
    output: T,
    pending: Option<T>,
}

impl<T: Copy> Dff<T> {
    pub fn new(initial: T) -> Self {
        Self {
            output: initial,
            pending: None,
        }
    }

    /// Read Q — the most recently captured value. Pending writes from
    /// this edge are invisible until `tick` fires.
    pub fn output(&self) -> T {
        self.output
    }

    /// Set the D input. Does not update Q.
    pub fn write(&mut self, value: T) {
        self.pending = Some(value);
    }

    /// Clock edge — capture D → Q if a write has been queued since the
    /// last tick.
    pub fn tick(&mut self) {
        if let Some(value) = self.pending.take() {
            self.output = value;
        }
    }

    /// Bypass the pending slot and set Q immediately. Used for reset /
    /// boot state or for cases where no capture-edge semantics apply.
    pub fn write_immediate(&mut self, value: T) {
        self.output = value;
        self.pending = None;
    }
}
