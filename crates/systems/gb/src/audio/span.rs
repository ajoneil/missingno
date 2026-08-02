//! Inert-span prediction: how many upcoming APU ticks the event model proves
//! produce neither a frame-sequencer strobe nor a change to the mixed output.
//!
//! Every bound here is a lower bound — a span may end earlier than the first
//! real event, never later.

use super::{ApuSpec, Audio, DIV_APU_BIT};

/// Longest span the predictor will claim.
const SPAN_CAP: u32 = 4096;

/// The prediction in flight, and the clock inputs the next tick must carry for
/// it to hold.
#[derive(Clone, Default)]
pub struct SpanPredictor {
    inert_ticks: u32,
    expected_div_counter: u16,
    expected_t_index: u8,
}

impl SpanPredictor {
    /// Whether this tick falls inside a proven-inert span. Clock inputs that
    /// deviate from the prediction — the blackout's held edges, a DIV write, a
    /// speed switch — drop the span instead of being asserted on.
    pub(super) fn consume(&mut self, div_counter: u16, t_index: u8) -> bool {
        if self.inert_ticks == 0 {
            return false;
        }
        if div_counter != self.expected_div_counter || t_index != self.expected_t_index {
            self.inert_ticks = 0;
            return false;
        }
        self.inert_ticks -= 1;
        self.expect_after(div_counter, t_index);
        true
    }

    pub(super) fn armed(&self) -> bool {
        self.inert_ticks > 0
    }

    /// Drop the prediction: an unpredictable mutation landed.
    pub(super) fn invalidate(&mut self) {
        self.inert_ticks = 0;
    }

    pub(super) fn arm(&mut self, inert_ticks: u32, div_counter: u16, t_index: u8) {
        self.inert_ticks = inert_ticks.min(SPAN_CAP);
        self.expect_after(div_counter, t_index);
    }

    /// The M-cycle counter advances with the T-index, so the tick after the one
    /// carrying `(div_counter, t_index)` has determined clock inputs.
    fn expect_after(&mut self, div_counter: u16, t_index: u8) {
        let next = (t_index + 1) & 0b11;
        self.expected_t_index = next;
        self.expected_div_counter = if next == 0 {
            div_counter.wrapping_add(1)
        } else {
            div_counter
        };
    }
}

/// Ticks ahead of the tick carrying `t_index` that the next tick carrying
/// `target` lands on (1..=4).
fn ticks_to_t_index(t_index: u8, target: u8) -> u32 {
    ((target + 4 - t_index - 1) % 4) as u32 + 1
}

/// Inert ticks following the tick carrying `(div_counter, t_index)` — the min
/// over every span-ending event. The APU must be powered and at single speed.
pub(super) fn predict_inert_ticks<A: ApuSpec>(
    audio: &Audio<A>,
    div_counter: u16,
    t_index: u8,
) -> u32 {
    let ch = &audio.channels;
    // A strobe's effects land after this tick's drain, so a flag still standing
    // recomputes the mix on the next tick.
    if ch.ch1.output_dirty || ch.ch2.output_dirty || ch.ch3.output_dirty || ch.ch4.output_dirty {
        return 0;
    }
    let mut span = ticks_to_frame_sequencer_strobe(audio, div_counter, t_index);

    // A sweep arm awaiting its BEXA drain, or a trigger's adder restart, resolves
    // at the next ajer↑ with effects the closed forms below do not cover.
    if ch.ch1.coze || ch.ch1.sweep_calc_restart {
        return 0;
    }
    // The adder's step counter advances once per M-cycle at ajer↑ and clears
    // `enabled` when it saturates on an overflow.
    if ch.ch1.sweep_calc_steps > 0 {
        let steps = ch.ch1.sweep_calc_steps as u32;
        span = span.min(ticks_to_t_index(t_index, 0) + (steps - 1) * 4 - 1);
    }

    if ch.ch1.enabled.enabled && ch.ch1.dac_enabled() {
        span = span.min(ticks_to_duty_change(
            t_index,
            ch.ch1.divider.counter,
            ch.ch1.period.0,
            ch.ch1.wave_duty_position,
            ch.ch1.ch1_frst,
            ch.ch1.pwm_latch,
            ch.ch1.pending_reload != super::channels::TriggerReload::Idle
                || ch.ch1.divider_load_settle,
            |position| ch.ch1.duty_bit(position),
        ));
    }
    if ch.ch2.enabled.enabled && ch.ch2.dac_enabled() {
        span = span.min(ticks_to_duty_change(
            t_index,
            ch.ch2.divider.counter,
            ch.ch2.period.0,
            ch.ch2.wave_duty_position,
            ch.ch2.ch2_frst,
            ch.ch2.pwm_latch,
            ch.ch2.pending_reload != super::channels::TriggerReload::Idle
                || ch.ch2.divider_load_settle,
            |position| ch.ch2.duty_bit(position),
        ));
    }
    if ch.ch3.enabled.enabled && ch.ch3.dac_enabled {
        span = span.min(ticks_to_wave_step(&ch.ch3));
    }
    if ch.ch4.enabled.enabled && ch.ch4.dac_enabled() {
        span = span.min(ticks_to_lfsr_shift(&ch.ch4));
    }

    span.min(SPAN_CAP)
}

/// Ticks to the tick that detects the DIV-APU tap fall — the strobe itself
/// lands one tick later through the `fs_edge` staging.
fn ticks_to_frame_sequencer_strobe<A: ApuSpec>(
    audio: &Audio<A>,
    div_counter: u16,
    t_index: u8,
) -> u32 {
    if audio.fs_edge_pending || audio.fs_edge_predelay {
        return 0;
    }
    let period = (DIV_APU_BIT as u32) << 1;
    let phase = div_counter as u32 & (period - 1);
    let increments = if phase == 0 { period } else { period - phase };
    // The counter steps once per M-cycle, on the tick carrying t_index 0.
    ticks_to_t_index(t_index, 0) + (increments - 1) * 4 - 1
}

/// Ticks to the next pulse-divider overflow whose latched duty bit differs from
/// the one driving the DAC now.
#[allow(clippy::too_many_arguments)]
fn ticks_to_duty_change(
    t_index: u8,
    counter: u16,
    period: u16,
    position: u8,
    frst: bool,
    latch: bool,
    reload_in_flight: bool,
    duty_bit: impl Fn(u8) -> bool,
) -> u32 {
    // The divider clocks at CALO↑, once per M-cycle.
    let to_first_clock = ticks_to_t_index(t_index, 1);
    if reload_in_flight {
        return to_first_clock - 1;
    }
    let counter = counter & 0x7FF;
    let period = period & 0x7FF;
    // duwo/dome capture the pre-advance duty step, so an overflow with the step
    // advance already pending latches the following position.
    let base = position + u8::from(frst);
    let mut clocks = 0x800 - counter as u32;
    for step in 0..8 {
        if duty_bit(base.wrapping_add(step)) != latch {
            return to_first_clock + (clocks - 1) * 4 - 1;
        }
        clocks += 0x800 - period as u32;
    }
    u32::MAX
}

/// Ticks to CH3's next divider overflow — the head of the chain that steps the
/// wave position and relatches the sample byte a few ticks later.
fn ticks_to_wave_step(ch3: &super::channels::wave::WaveChannel) -> u32 {
    let sync = &ch3.trigger_sync;
    if sync.bit_latch || sync.armed || sync.restart {
        return 0;
    }
    let latch = &ch3.wave_data_latch;
    if ch3.ch3_frst || ch3.pending_overflow || latch.sync_1 || latch.sync_2 {
        return 0;
    }
    if latch.latched || latch.extended {
        return 0;
    }
    if ch3.ch3_fdis || ch3.frequency_timer == 0 {
        return u32::MAX;
    }
    // The divider counts on ch3_2mhz↑ — every other tick.
    let to_first_rise = if ch3.ch3_2mhz { 2 } else { 1 };
    to_first_rise + (ch3.frequency_timer as u32 - 1) * 2 - 1
}

/// Ticks to CH4's next LFSR shift: the tapped divider bit's next rise, over the
/// slowest cadence the divisor code can produce.
fn ticks_to_lfsr_shift(ch4: &super::channels::noise::NoiseChannel) -> u32 {
    if ch4.sync_delay > 0 {
        return ch4.sync_delay as u32;
    }
    let shift = ch4.frequency_and_randomness.clock_shift();
    if shift >= 14 {
        return u32::MAX;
    }
    let mask = (1u32 << (shift + 1)) - 1;
    let half = 1u32 << shift;
    let phase = ch4.divider as u32 & mask;
    let mut increments = (half.wrapping_sub(phase)) & mask;
    if increments == 0 {
        increments = mask + 1;
    }
    // gary lets the divider through once per 4 T on code 0, once per 8·code
    // otherwise; the subcounter's remaining count bounds the first one.
    let code = ch4.frequency_and_randomness.divisor_code() as u32;
    let cadence = if code == 0 { 4 } else { 8 * code };
    let to_first = (ch4.divider_subcounter as u32).max(1);
    to_first + (increments - 1) * cadence - 1
}
