//! TIA audio: two independent channels, each the real hardware datapath — a
//! 4-bit pulse counter and a 5-bit noise LFSR, both gated by an AUDF ÷(N+1)
//! divider, with the AUDC register split into a pulse-feedback selector (high
//! 2 bits) and a pulse-hold/noise-chain gate (low 2 bits). Each audio tick is
//! a two-phase pair: phase0 samples from pre-tick state, phase1 commits. The
//! waveforms emerge from the structure; there are no per-mode tables.

/// The 5-bit noise LFSR; bit j holds n_j — bit 4 the newest (shift-in) end,
/// bit 0 the oldest. Feedback n2 ⊕ n0 gives the inserted-bit recurrence
/// s[i] = s[i-3] ⊕ s[i-5] (period 31).
#[derive(Clone, Copy)]
struct NoiseCounter {
    reg: u8,
}

impl NoiseCounter {
    /// Power-on contents are indeterminate; seeded with the AUDC=$00 rest
    /// state (all ones), which the silence decode drains any state to.
    fn new() -> Self {
        NoiseCounter { reg: 0x1F }
    }

    /// The oldest bit n0 — the shift-out end feeding the tap latch. (N1501)
    fn oldest(&self) -> bool {
        self.reg & 1 != 0
    }

    /// The middle feedback tap n2 (the one the 9-bit chain swaps out). (N1039)
    fn mid_tap(&self) -> bool {
        (self.reg >> 2) & 1 != 0
    }

    fn all_zero(&self) -> bool {
        self.reg == 0
    }

    /// The gated-÷31 advance window: (n4,n3,n2,n1) = (0,0,0,1), n0 ignored —
    /// two states of the 31, so two pulse advances per noise period. (N2237)
    fn div31_window(&self) -> bool {
        self.reg & 0x1E == 0x02
    }

    fn commit(&mut self, feedback: bool) {
        self.reg = (self.reg >> 1) | ((feedback as u8) << 4);
    }
}

/// The 4-bit pulse counter; bit j holds p_j — feedback enters at p3 and
/// values shift through unchanged (static cells). The LSB p0 switches the DAC.
#[derive(Clone, Copy)]
struct PulseCounter {
    reg: u8,
}

impl PulseCounter {
    /// Power-on contents are indeterminate; seeded with the AUDC=$00 rest
    /// state (all zeros), which the grounded feedback mux drains any state to.
    fn new() -> Self {
        PulseCounter { reg: 0x00 }
    }

    /// The all-ones state decode.
    fn all_ones(&self) -> bool {
        self.reg == 0x0F
    }

    fn lsb(&self) -> bool {
        self.reg & 1 != 0
    }

    fn bit1(&self) -> bool {
        (self.reg >> 1) & 1 != 0
    }

    fn bit2(&self) -> bool {
        (self.reg >> 2) & 1 != 0
    }

    fn top(&self) -> bool {
        (self.reg >> 3) & 1 != 0
    }

    fn commit(&mut self, feedback: bool) {
        self.reg = (self.reg >> 1) | ((feedback as u8) << 3);
    }
}

/// The AUDF frequency divider: a 5-bit up-counter compared to AUDF. On the
/// match it asserts the clock-enable and reloads to 0, so the enable fires once
/// every AUDF+1 ticks.
#[derive(Clone, Copy)]
struct AudfDivider {
    count: u8,
}

impl AudfDivider {
    fn new() -> Self {
        AudfDivider { count: 0 }
    }

    fn tick(&mut self, audf: u8) -> bool {
        let enable = self.count == audf & 0x1F;
        self.count = if enable { 0 } else { self.count + 1 };
        enable
    }
}

pub struct Channel {
    /// AUDC waveform select, AUDF frequency divisor, AUDV volume.
    pub control: u8,
    pub frequency: u8,
    pub volume: u8,
    divider: AudfDivider,
    pulse: PulseCounter,
    noise: NoiseCounter,
    /// The divider's clock-enable, latched at phase0 for phase1.
    enable: bool,
    /// The noise shift-in, sampled at phase0 from pre-tick state.
    noise_feedback: bool,
    /// The pre-shift oldest noise bit, latched at phase0 — the buffered tap
    /// the pulse-side feedback and hold decodes read. (N2536 half-stage)
    noise_tap: bool,
    /// The pulse-hold decision, latched at phase0. (N1530)
    advance: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel {
    pub fn new() -> Self {
        Channel {
            control: 0,
            frequency: 0,
            volume: 0,
            divider: AudfDivider::new(),
            pulse: PulseCounter::new(),
            noise: NoiseCounter::new(),
            enable: false,
            noise_feedback: false,
            noise_tap: false,
            advance: false,
        }
    }

    /// phase0 — the sample phase: the divider compares, and the noise
    /// shift-in, the noise tap, and the pulse-hold decision all latch from
    /// pre-tick state.
    pub fn phase0(&mut self) {
        self.enable = self.divider.tick(self.frequency);
        if !self.enable {
            return;
        }
        self.noise_tap = self.noise.oldest();
        self.advance = match self.control & 0x03 {
            0 | 1 => true,
            2 => self.noise.div31_window(),
            _ => self.noise_tap,
        };
        // Low-2 = 0 swaps the n2 tap for ¬(pulse LSB), chaining the two
        // counters into the 511-state loop. (tap mux N661)
        let tap = if self.control & 0x03 == 0 {
            !self.pulse.lsb()
        } else {
            self.noise.mid_tap()
        };
        // Grounding the feedback hub inserts a 1: the all-low escape (N781;
        // the pulse all-ones decode N820 confines it to the chained lock
        // state in low-2 = 0) and the AUDC=$00 silence decode (N1632).
        let escape = self.noise.all_zero() && (self.control & 0x03 != 0 || self.pulse.all_ones());
        let silence = self.control & 0x0F == 0;
        self.noise_feedback = escape || silence || (tap ^ self.noise.oldest());
    }

    /// phase1 — the commit phase: the noise shift lands, then the pulse
    /// captures its AUDC-selected feedback from pre-advance values and the
    /// latched tap.
    pub fn phase1(&mut self) {
        if !self.enable {
            return;
        }
        self.noise.commit(self.noise_feedback);
        if !self.advance {
            return;
        }
        // AUDC=$00 grounds the feedback mux (N1632 on N2203), parking the
        // counter at zero — the output rests at the conducting level.
        let feedback = self.control & 0x0F != 0
            && match (self.control >> 2) & 0x03 {
                // 4-bit poly ¬(p1 ⊕ p0) (N1462); the all-ones escape is
                // decap-asserted (hardware-undecided analog corner).
                0 => !(self.pulse.bit1() ^ self.pulse.lsb()) && !self.pulse.all_ones(),
                // ÷2 square: the inverted top bit re-enters. (N544)
                1 => !self.pulse.top(),
                // follow-noise: the complemented tap. (N1810)
                2 => !self.noise_tap,
                // ÷3 (N1820 gives ¬p1; the (¬p2 ∨ p3) basin-drain factor is
                // decap-asserted, hardware-undecided analog corner).
                _ => !self.pulse.bit1() && (!self.pulse.bit2() || self.pulse.top()),
            };
        self.pulse.commit(feedback);
    }

    /// The conducting DAC legs in units of the smallest, 0-15: the legs
    /// conduct while the pulse LSB is low, so AUDV's binary weights sum to
    /// ¬LSB × AUDV. AUDC=$00 parks the LSB low, making AUDV a constant DC
    /// level (the sample-playback path).
    pub fn conductance(&self) -> u8 {
        if self.pulse.lsb() { 0 } else { self.volume }
    }
}

/// D0's leg; D1, D2 and D3 are 15K, 7.5K and 3.75K, so a channel's legs sum
/// to its AUDV value in units of this one's conductance.
const LSB_LEG_OHMS: f32 = 30_000.0;
/// The board ties both audio pads together and to this single pull-up — the
/// die has none, so the two channels share one summing node.
const PULLUP_OHMS: f32 = 1_000.0;
/// Both channels conducting every leg.
const FULL_SCALE_CONDUCTANCE: f32 = 30.0;

/// The shared node's excursion from its resting level, as a fraction of the
/// supply. Conducting pulls the node down against the pull-up, so the
/// excursion saturates: each leg switched on wins a smaller share of the
/// divider than the last.
fn node_excursion(conductance: f32) -> f32 {
    let legs = conductance * PULLUP_OHMS / LSB_LEG_OHMS;
    legs / (1.0 + legs)
}

/// The summing node's level, 0.0-1.0, for the two channels' combined
/// conductance. Because they share the node, a second channel adds less than
/// the first, and only the total matters — equal AUDV sums sound alike.
/// Full scale is taken at both channels wide open, which is a convention.
pub(crate) fn summing_node_level(conductance: u8) -> f32 {
    node_excursion(f32::from(conductance)) / node_excursion(FULL_SCALE_CONDUCTANCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a channel at AUDF=0 (enable every tick) and collect the pulse
    /// LSB (the die output node) sampled after each phase1, for `n` ticks.
    fn run(control: u8, n: usize) -> Vec<u8> {
        let mut ch = Channel::new();
        ch.control = control;
        ch.volume = 0x0F;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            ch.phase0();
            ch.phase1();
            out.push(ch.pulse.lsb() as u8);
        }
        out
    }

    fn period(seq: &[u8]) -> Option<usize> {
        let s = &seq[40..];
        (1..s.len() / 2).find(|&p| (0..s.len() - p).all(|i| s[i] == s[i + p]))
    }

    fn ones_per_period(seq: &[u8], p: usize) -> usize {
        seq[40..40 + p].iter().map(|&b| b as usize).sum()
    }

    #[test]
    fn follow_noise_is_the_inverted_m_sequence() {
        // AUDC=$09: the output is the complemented, delayed noise m-sequence —
        // period 31, 15 ones, recurrence o[i] = ¬(o[i-3] ⊕ o[i-5]).
        let seq = run(0x09, 200);
        assert_eq!(period(&seq), Some(31));
        assert_eq!(ones_per_period(&seq, 31), 15);
        for i in 45..seq.len() {
            assert_eq!(seq[i], 1 - (seq[i - 3] ^ seq[i - 5]));
        }
    }

    #[test]
    fn poly4_is_the_inverted_recurrence() {
        // AUDC=$01: period 15, recurrence o[i] = ¬(o[i-3] ⊕ o[i-4]).
        let seq = run(0x01, 200);
        assert_eq!(period(&seq), Some(15));
        for i in 45..seq.len() {
            assert_eq!(seq[i], 1 - (seq[i - 3] ^ seq[i - 4]));
        }
    }

    #[test]
    fn pure_tone_is_div2() {
        // AUDC=$04 (÷2 square): the LSB toggles every tick, period 2.
        assert_eq!(period(&run(0x04, 100)), Some(2));
    }

    #[test]
    fn nine_bit_noise_is_511() {
        // AUDC=$08: the low-2 = 0 tap swap chains pulse ↔ noise into the
        // 511-state loop; recurrence o[i] = ¬(o[i-5] ⊕ o[i-9]).
        let seq = run(0x08, 1400);
        assert_eq!(period(&seq), Some(511));
        for i in 49..seq.len() {
            assert_eq!(seq[i], 1 - (seq[i - 5] ^ seq[i - 9]));
        }
    }

    #[test]
    fn div6_tone_is_period_6() {
        // AUDC=$0C (÷3 feedback, free-run): the ÷6 tone.
        assert_eq!(period(&run(0x0C, 120)), Some(6));
    }

    #[test]
    fn gated_div31_has_period_465() {
        // AUDC=$02: the pulse advances twice per noise period (the masked
        // window decode), giving the 465-tick output period.
        assert_eq!(period(&run(0x02, 1400)), Some(465));
    }

    #[test]
    fn mode_07_differs_from_09_same_period() {
        // ÷2-family orbits come in complementary pairs (feedback ¬p3 is
        // complement-equivariant), so $07's output sense is power-on-dependent;
        // only its period and shape-vs-$09 are absolute.
        let a = run(0x07, 200);
        let b = run(0x09, 200);
        assert_eq!(period(&a), Some(31));
        assert_eq!(period(&b), Some(31));
        assert_ne!(&a[40..71], &b[40..71]);
    }

    #[test]
    fn silence_outputs_the_volume_as_dc() {
        // AUDC=$00: the grounded feedback mux parks the LSB low within a few
        // ticks; the DAC then conducts constantly — level = AUDV.
        let mut ch = Channel::new();
        ch.control = 0x00;
        ch.volume = 0x0F;
        for _ in 0..8 {
            ch.phase0();
            ch.phase1();
        }
        for _ in 0..50 {
            ch.phase0();
            ch.phase1();
            assert_eq!(ch.conductance(), 0x0F);
        }
    }

    #[test]
    fn only_the_combined_conductance_reaches_the_node() {
        // The pads share one node, so equal AUDV sums are indistinguishable.
        let both_wide = summing_node_level(8 + 8);
        assert_eq!(summing_node_level(15 + 1), both_wide);
        assert_eq!(summing_node_level(10 + 6), both_wide);
    }

    #[test]
    fn a_second_channel_adds_less_than_the_first() {
        // Conducting divides against the pull-up, so the node saturates: one
        // channel wide open already reaches two thirds of full scale.
        assert!((summing_node_level(15) - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(summing_node_level(0), 0.0);
        assert_eq!(summing_node_level(30), 1.0);
    }

    #[test]
    fn audf_divides_by_n_plus_1() {
        // AUDF=1 → the waveform updates half as often; a ÷2 tone's period doubles.
        let mut ch = Channel::new();
        ch.control = 0x04;
        ch.frequency = 1;
        ch.volume = 0x0F;
        let mut out = Vec::new();
        for _ in 0..100 {
            ch.phase0();
            ch.phase1();
            out.push(ch.pulse.lsb() as u8);
        }
        assert_eq!(period(&out), Some(4));
    }
}
