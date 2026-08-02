/// The per-channel frame-sequencer length counter. `MAX` is the reload
/// ceiling — 64 on CH1/CH2/CH4, 256 on CH3. Owns the NRx4 length-enable
/// latch and the down-counter; the NRx4 enable-rising extra clock and the
/// trigger-coincident MAX→MAX-1 fixup are corpus-pinned hardware glitches.
#[derive(Clone, Default)]
#[cfg_attr(debug_assertions, derive(Debug, PartialEq))]
pub struct LengthCounter<const MAX: u16> {
    pub enabled: bool,
    pub counter: u16,
}

impl<const MAX: u16> LengthCounter<MAX> {
    /// NRx1/NR31 length-timer write: reload to `MAX - value`, where `value`
    /// is the register's length field (6-bit on CH1/2/4, 8-bit on CH3).
    pub fn load(&mut self, value: u16) {
        self.counter = MAX - value;
    }

    /// Frame-sequencer length step (caru↓). Returns true when the counter
    /// reaches 0 this step — the caller disables the channel.
    pub fn tick(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            self.counter == 0
        } else {
            false
        }
    }

    /// NRx4 length-enable rising edge (deme/capy/gepy/doda): a 0→1 enable
    /// while `caru` is low clocks one extra length count. Returns true when
    /// that clock reaches 0 on a non-trigger write — the caller disables.
    pub fn enable_glitch(&mut self, caru_low: bool, enable_length: bool, trigger: bool) -> bool {
        let was_enabled = self.enabled;
        self.enabled = enable_length;
        if caru_low && !was_enabled && self.enabled && self.counter > 0 {
            self.counter -= 1;
            if self.counter == 0 && !trigger {
                return true;
            }
        }
        false
    }

    /// Trigger reload: an expired counter reloads to `MAX`.
    pub fn trigger_reload(&mut self) {
        if self.counter == 0 {
            self.counter = MAX;
        }
    }

    /// Trigger-coincident enable fixup: a trigger that reloads to `MAX` while
    /// length-enabled and `caru` low immediately clocks once (MAX→MAX-1).
    pub fn trigger_enable_fixup(&mut self, caru_low: bool) {
        if caru_low && self.enabled && self.counter == MAX {
            self.counter = MAX - 1;
        }
    }
}
