use super::registers::EnvelopeDirection;

/// The per-channel volume envelope shared by CH1/CH2/CH4: the volume
/// counter, its step timer, the `kyvo` saturation arm (sampled into
/// JOPA/KOZY), and the `JEME` stop latch. The NRx2 pace and direction
/// live in the channel register and are passed in per operation. The
/// enable-bug arm is used by the two pulse channels; CH4 never sets it.
#[derive(Clone, Default)]
pub struct Envelope {
    pub volume: u8,
    pub timer: u8,
    /// `kyvo` — envelope-counter saturation arm. Set at kene↓; sampled
    /// into JOPA/KOZY on the next horu_512hz↑.
    pub saturation_armed: bool,
    /// `JEME` stop latch: a fire that samples a saturated counter latches
    /// it, pinning HOFO until the next trigger clears it.
    pub stopped: bool,
    /// Envelope-enable bug: an NRx2 write that turns the envelope on
    /// (pace 0→non-zero) makes the next even DIV-APU tick advance the
    /// counter. Pulse channels only; never armed on CH4.
    pub enable_tick_pending: bool,
}

impl Envelope {
    /// Trigger reload: volume/timer from NRx2, clear JEME and any kyvo arm.
    pub fn trigger(&mut self, initial_volume: u8, pace: u8) {
        self.volume = initial_volume;
        self.timer = pace;
        self.stopped = false;
        self.saturation_armed = false;
    }

    /// Write-strobe transient: the pace bits read 1 while the cells settle,
    /// so a write whose *old* pace was 0 clocks the volume once (free 4-bit
    /// wrap; JEME never latches under pace 0).
    pub fn zombie_bump(&mut self, old_pace: u8) {
        if old_pace == 0 && !self.stopped {
            self.volume = (self.volume + 1) & 0xf;
        }
    }

    /// kene↓: advance the counter, arm `kyvo` on saturation. `held` is the
    /// divider load-settle window (pulse only; false on CH4) — dmg_tffnl
    /// holds the counter while it is open.
    pub fn tick_counter(&mut self, pace: u8, held: bool) {
        if held || pace == 0 {
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = pace;
            self.saturation_armed = true;
        }
    }

    /// horu_512hz↑ JOPA/KOZY sample: drain an armed `kyvo` into the volume
    /// counter, or latch JEME on a saturated counter. Returns true when the
    /// volume actually stepped (the caller's `output_dirty`).
    pub fn sample_fire(
        &mut self,
        pace: u8,
        channel_enabled: bool,
        direction: EnvelopeDirection,
    ) -> bool {
        if !self.saturation_armed {
            return false;
        }
        self.saturation_armed = false;
        if pace == 0 || !channel_enabled || self.stopped {
            return false;
        }
        // HEPO captures the saturation decode at the fire: a saturated
        // counter latches JEME instead of stepping — no arithmetic clamp.
        match direction {
            EnvelopeDirection::Increase => {
                if self.volume == 15 {
                    self.stopped = true;
                    false
                } else {
                    self.volume += 1;
                    true
                }
            }
            EnvelopeDirection::Decrease => {
                if self.volume == 0 {
                    self.stopped = true;
                    false
                } else {
                    self.volume -= 1;
                    true
                }
            }
        }
    }

    /// Consume the enable-bug arm set by the last enabling NRx2 write; the
    /// caller advances the counter on the even tick.
    pub fn take_enable_tick_pending(&mut self) -> bool {
        let pending = self.enable_tick_pending;
        self.enable_tick_pending = false;
        pending
    }
}
