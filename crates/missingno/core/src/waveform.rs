//! The debugger's per-channel waveform surface: a rolling capture of the
//! digital codes each sound channel hands its DAC, as the silicon produced
//! them. Analog shaping (the DAC transfer curve, the board's coupling) is
//! frontend policy — the codes here stop at the DAC's input, the off-chip
//! boundary. A core fills a [`WaveRing`] at its own audio cadence while capture
//! is enabled and reads it back as [`ChannelWave`]s.

/// One channel's captured waveform: the DAC input codes it drove, oldest first.
#[derive(Clone, Debug)]
pub struct ChannelWave {
    /// The channel's display name ("CH1", "CH0", ...).
    pub label: &'static str,
    /// DAC input codes, oldest first — one per capture tick.
    pub levels: Vec<u8>,
    /// Bits of resolution in each code (4 for the Game Boy channels and the
    /// TIA's AUDx legs).
    pub depth_bits: u8,
    /// The capture cadence in Hz — the rate the core samples codes at.
    pub rate: u32,
    /// Whether the channel's DAC was driving at the end of the window.
    pub active: bool,
}

/// A fixed-capacity rolling buffer of DAC input codes: pushing past capacity
/// overwrites the oldest code, so it always holds the most recent window.
#[derive(Clone, Debug)]
pub struct WaveRing {
    buf: Box<[u8]>,
    /// The next write index.
    head: usize,
    /// Valid codes held, saturating at capacity.
    len: usize,
}

impl WaveRing {
    /// A ring holding up to `capacity` codes.
    pub fn new(capacity: usize) -> Self {
        WaveRing {
            buf: vec![0; capacity.max(1)].into_boxed_slice(),
            head: 0,
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append a code, overwriting the oldest once full.
    pub fn push(&mut self, code: u8) {
        let cap = self.buf.len();
        self.buf[self.head] = code;
        self.head = (self.head + 1) % cap;
        self.len = (self.len + 1).min(cap);
    }

    /// Drop every held code; the ring reads empty until refilled.
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// The held codes, oldest first.
    pub fn to_vec(&self) -> Vec<u8> {
        let cap = self.buf.len();
        if self.len < cap {
            self.buf[..self.len].to_vec()
        } else {
            let mut out = Vec::with_capacity(cap);
            out.extend_from_slice(&self.buf[self.head..]);
            out.extend_from_slice(&self.buf[..self.head]);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WaveRing;

    #[test]
    fn empty_ring_reads_empty() {
        let ring = WaveRing::new(4);
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.to_vec(), Vec::<u8>::new());
    }

    #[test]
    fn fills_oldest_first_until_capacity() {
        let mut ring = WaveRing::new(4);
        ring.push(1);
        ring.push(2);
        ring.push(3);
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn wraps_dropping_the_oldest() {
        let mut ring = WaveRing::new(4);
        for code in 1..=6 {
            ring.push(code);
        }
        // Capacity 4: the two oldest (1, 2) were overwritten.
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.to_vec(), vec![3, 4, 5, 6]);
    }

    #[test]
    fn wrap_ordering_holds_across_the_seam() {
        let mut ring = WaveRing::new(3);
        // head lands mid-buffer, so oldest-first must stitch the two halves.
        for code in [10, 20, 30, 40, 50] {
            ring.push(code);
        }
        assert_eq!(ring.to_vec(), vec![30, 40, 50]);
    }

    #[test]
    fn clear_empties_the_window() {
        let mut ring = WaveRing::new(4);
        ring.push(7);
        ring.push(8);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.to_vec(), Vec::<u8>::new());
        // Refilling after a clear starts fresh at the oldest slot.
        ring.push(9);
        assert_eq!(ring.to_vec(), vec![9]);
    }

    #[test]
    fn zero_capacity_is_clamped_to_one() {
        let mut ring = WaveRing::new(0);
        ring.push(5);
        ring.push(6);
        assert_eq!(ring.to_vec(), vec![6]);
    }
}
