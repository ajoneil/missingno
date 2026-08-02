//! Inert-span prediction: how many upcoming APU ticks the event model proves
//! produce neither a frame-sequencer strobe nor a change to the mixed output.
//!
//! A span also has to be *reconstructible*: the prediction is 0 whenever a
//! channel carries pipeline residue the materialisation does not model, so
//! every armed span lands on state closed forms can reproduce exactly.

use super::channels::{TriggerReload, noise::NoiseChannel, wave::WaveChannel};
use super::{ApuSpec, Audio, DIV_APU_BIT, DIV_APU_BIT_DOUBLE};

/// Longest span the predictor will claim.
const SPAN_CAP: u32 = 4096;

/// Shortest span worth skipping: below it the slow path runs to the event, so
/// adversarial content pays one compare per tick and nothing else.
pub(super) const THRESHOLD: u32 = 16;

/// Shared-prescaler phase carrying CALO↑ (chN_1mhz↑) — the 1 MHz clock the
/// pulse dividers count on and the one `jeso` flips with.
pub(super) const CLOCK_RISE_PHASE: u8 = 2;

/// Shared-prescaler phase carrying ajer↑ — the sweep adder's step.
pub(super) const SWEEP_STEP_PHASE: u8 = 1;

/// What a tick's span lookup resolved to.
pub(super) enum Consumed {
    /// Outside any span — the slow path owns this tick.
    Miss,
    /// Inside a span the predictor claimed but is not skipping.
    Inert,
    /// Inside a skipped span: the tick's work is deferred to materialisation.
    Skipped,
}

/// The prediction in flight, and the clock inputs the next tick must carry for
/// it to hold.
#[derive(Clone, Default)]
pub struct SpanPredictor {
    inert_ticks: u32,
    /// Ticks skipped so far in this span, awaiting materialisation.
    skipped: u32,
    skipping: bool,
    skipped_last_tick: bool,
    /// The KEY1 regime the span was armed in. It fixes the tick cadence and the
    /// DIV-APU tap, and a speed switch invalidates, so it holds span-wide.
    double_speed: bool,
    last_div_counter: u16,
    expected_div_counter: u16,
    expected_t_index: u8,
}

impl SpanPredictor {
    /// Whether this tick falls inside a proven-inert span. Clock inputs that
    /// deviate from the prediction — the blackout's held edges, a DIV write, a
    /// speed switch — drop the span instead of being asserted on.
    pub(super) fn consume(&mut self, div_counter: u16, t_index: u8) -> Consumed {
        self.skipped_last_tick = false;
        if self.inert_ticks == 0 {
            return Consumed::Miss;
        }
        if div_counter != self.expected_div_counter || t_index != self.expected_t_index {
            self.inert_ticks = 0;
            return Consumed::Miss;
        }
        self.inert_ticks -= 1;
        self.expect_after(div_counter, t_index);
        if !self.skipping {
            return Consumed::Inert;
        }
        self.skipped += 1;
        self.last_div_counter = div_counter;
        self.skipped_last_tick = true;
        Consumed::Skipped
    }

    pub(super) fn armed(&self) -> bool {
        self.inert_ticks > 0
    }

    /// The dot fall belonging to a skipped rise is skipped with it.
    pub(super) fn skipped_last_tick(&self) -> bool {
        self.skipped_last_tick
    }

    /// Ticks skipped and not yet reconstructed.
    pub(super) fn skipped(&self) -> u32 {
        self.skipped
    }

    /// Drop the prediction: an unpredictable mutation landed.
    pub(super) fn invalidate(&mut self) {
        self.inert_ticks = 0;
        self.skipped_last_tick = false;
    }

    /// The skipped run to reconstruct: `(ticks, the last tick's divider
    /// counter, and the regime the span was armed in)`.
    pub(super) fn take_skipped(&mut self) -> (u32, u16, bool) {
        let run = (self.skipped, self.last_div_counter, self.double_speed);
        self.skipped = 0;
        self.invalidate();
        run
    }

    pub(super) fn arm(&mut self, inert_ticks: u32, div_counter: u16, t_index: u8, ds: bool) {
        self.inert_ticks = inert_ticks.min(SPAN_CAP);
        self.double_speed = ds;
        // Skipping stays off at ÷2 until the regime's event model is proven.
        self.skipping = self.inert_ticks >= THRESHOLD && !ds;
        self.skipped = 0;
        self.expect_after(div_counter, t_index);
    }

    /// The clock inputs the tick after the one carrying `(div_counter,
    /// t_index)` must deliver. At ÷2 the APU is ticked on alternate CPU
    /// T-cycles, so the index steps in twos and the M-boundary increment lands
    /// after the `t∈{2,3}` call rather than after t 3.
    fn expect_after(&mut self, div_counter: u16, t_index: u8) {
        let (step, last_before_boundary) = if self.double_speed { (2, 2) } else { (1, 3) };
        self.expected_t_index = (t_index + step) & 0b11;
        self.expected_div_counter = if t_index >= last_before_boundary {
            div_counter.wrapping_add(1)
        } else {
            div_counter
        };
    }
}

/// Ticks ahead of the tick carrying `t_index` that the next tick carrying
/// `target` lands on (1..=4). Single speed only — at ÷2 the delivered index
/// steps in twos.
fn ticks_to_t_index(t_index: u8, target: u8) -> u32 {
    ((target + 4 - t_index - 1) % 4) as u32 + 1
}

/// Ticks ahead of the tick leaving the shared prescaler at `phase` that the
/// next tick reaching `target` lands on (1..=4). The prescaler advances once
/// per tick in both regimes, so this is the speed-independent M-phase clock.
fn ticks_to_phase(phase: u8, target: u8) -> u32 {
    ((target + 4 - phase - 1) % 4) as u32 + 1
}

/// How many of the `ticks` starting at prescaler phase `entry` reach `target`.
pub(super) fn phase_count(entry: u8, target: u8, ticks: u32) -> u32 {
    let offset = ((target + 4 - entry) % 4) as u32;
    if offset >= ticks {
        0
    } else {
        (ticks - offset).div_ceil(4)
    }
}

/// Inert ticks following the tick carrying `(div_counter, t_index)` — the min
/// over every span-ending event. The APU must be powered.
pub(super) fn predict_inert_ticks<A: ApuSpec>(
    audio: &Audio<A>,
    div_counter: u16,
    t_index: u8,
    double_speed: bool,
) -> u32 {
    let ch = &audio.channels;
    // Every M-phase event is a phase of the shared prescaler, which this tick
    // has already advanced.
    let clock_phase = audio.channel_clock.counter;
    // A strobe's effects land after this tick's drain, so a flag still standing
    // recomputes the mix on the next tick.
    if ch.ch1.output_dirty || ch.ch2.output_dirty || ch.ch3.output_dirty || ch.ch4.output_dirty {
        return 0;
    }
    let mut span = ticks_to_frame_sequencer_strobe(audio, div_counter, t_index, double_speed);

    // A sweep arm awaiting its BEXA drain, or a trigger's adder restart, resolves
    // at the next ajer↑ with effects the closed forms below do not cover.
    if ch.ch1.coze || ch.ch1.sweep_calc_restart {
        return 0;
    }
    // The adder's step counter advances once per M-cycle at ajer↑ and clears
    // `enabled` when it saturates on an overflow.
    if ch.ch1.sweep_calc_steps > 0 {
        let steps = ch.ch1.sweep_calc_steps as u32;
        span = span.min(ticks_to_phase(clock_phase, SWEEP_STEP_PHASE) + (steps - 1) * 4 - 1);
    }

    // A divider reload in flight moves the duty pipeline off its steady cadence.
    if ch.ch1.pending_reload != TriggerReload::Idle
        || ch.ch1.divider_load_settle
        || ch.ch2.pending_reload != TriggerReload::Idle
        || ch.ch2.divider_load_settle
    {
        return 0;
    }

    if ch.ch1.enabled.enabled {
        span = span.min(ticks_to_duty_change(
            clock_phase,
            ch.ch1.divider.counter,
            ch.ch1.period.0,
            ch.ch1.wave_duty_position,
            ch.ch1.ch1_frst,
            ch.ch1.pwm_latch,
            |position| ch.ch1.duty_bit(position),
        ));
    }
    if ch.ch2.enabled.enabled {
        span = span.min(ticks_to_duty_change(
            clock_phase,
            ch.ch2.divider.counter,
            ch.ch2.period.0,
            ch.ch2.wave_duty_position,
            ch.ch2.ch2_frst,
            ch.ch2.pwm_latch,
            |position| ch.ch2.duty_bit(position),
        ));
    }
    span = span.min(ticks_to_wave_step(&ch.ch3));
    if span == 0 {
        return 0;
    }
    span = span.min(ticks_to_noise_output_change(
        &ch.ch4,
        (clock_phase + 1) & 0b11,
        span.min(SPAN_CAP),
    ));

    span.min(SPAN_CAP)
}

/// Ticks to the tick that detects the DIV-APU tap fall — the strobe itself
/// lands one tick later through the `fs_edge` staging.
fn ticks_to_frame_sequencer_strobe<A: ApuSpec>(
    audio: &Audio<A>,
    div_counter: u16,
    t_index: u8,
    double_speed: bool,
) -> u32 {
    if audio.fs_edge_pending || audio.fs_edge_predelay {
        return 0;
    }
    let tap = if double_speed {
        DIV_APU_BIT_DOUBLE
    } else {
        DIV_APU_BIT
    };
    let period = (tap as u32) << 1;
    let tap_phase = div_counter as u32 & (period - 1);
    let increments = if tap_phase == 0 {
        period
    } else {
        period - tap_phase
    };
    // The divider steps once per M-cycle. At ÷1 the tick carrying t_index 0
    // observes it; at ÷2 only two of the four indices carry a tick, and the
    // increment lands between the `t∈{2,3}` tick and the next one.
    let (to_increment, ticks_per_increment) = if double_speed {
        (if t_index >= 2 { 1 } else { 2 }, 2)
    } else {
        (ticks_to_t_index(t_index, 0), 4)
    };
    to_increment + (increments - 1) * ticks_per_increment - 1
}

/// Ticks to the next pulse-divider overflow whose latched duty bit differs from
/// the one driving the DAC now. A disabled channel's divider is frozen, so this
/// is asked only of a running one — which the DAC-off write path guarantees is
/// also a contributing one.
#[allow(clippy::too_many_arguments)]
fn ticks_to_duty_change(
    phase: u8,
    counter: u16,
    period: u16,
    position: u8,
    frst: bool,
    latch: bool,
    duty_bit: impl Fn(u8) -> bool,
) -> u32 {
    // The divider clocks at CALO↑, once per M-cycle.
    let to_first_clock = ticks_to_phase(phase, CLOCK_RISE_PHASE);
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
/// wave position and relatches the sample byte a few ticks later. The whole
/// chain is a span-ender whether or not CH3 reaches the mix: its residue is
/// what materialisation cannot reconstruct, and a wave-RAM read observes the
/// position either way.
fn ticks_to_wave_step(ch3: &WaveChannel) -> u32 {
    let sync = &ch3.trigger_sync;
    if sync.bit_latch || sync.armed || sync.restart || sync.self_clear {
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

/// CH4's divider chain, walked at its own `ch4_1mhz` cadence (4 T per chain
/// tick) from a reference tick.
pub(super) struct NoiseChain {
    /// Ticks past the reference tick that the pending chain tick lands on.
    pub offset: u32,
    pub prescaler: u8,
    pub jeso: bool,
    pub divisor_code: u8,
}

impl NoiseChain {
    /// The chain as seen from the tick before the one leaving the shared
    /// prescaler at `entry_phase`, whose `jeso`, prescaler and subcounter are
    /// that tick's post-state.
    pub(super) fn new(ch4: &NoiseChannel, entry_phase: u8) -> Self {
        // A subcounter already at 0 reloads on the very next tick.
        let offset = (ch4.divider_subcounter as u32).max(1);
        // `jeso` flips on each ch4_1mhz↑ — exactly one of the four ticks
        // leading to a chain tick, so it alternates chain tick to chain tick
        // once the first flip is placed.
        let flipped = phase_count(entry_phase, CLOCK_RISE_PHASE, offset) == 1;
        Self {
            offset,
            prescaler: ch4.prescaler,
            jeso: ch4.jeso ^ flipped,
            divisor_code: ch4.frequency_and_randomness.divisor_code(),
        }
    }

    /// Take the pending chain tick, returning whether `gary` let the divider
    /// through on it.
    pub(super) fn step(&mut self) -> bool {
        if self.prescaler == 0b111 {
            self.prescaler = !self.divisor_code & 0b111;
        } else if !self.jeso {
            self.prescaler = (self.prescaler + 1) & 0b111;
        }
        self.prescaler == 0b111
    }

    /// Move on to the following chain tick (4 T later, one `jeso` flip on).
    pub(super) fn advance(&mut self) {
        self.offset += 4;
        self.jeso = !self.jeso;
    }

    /// T-cycles between divider increments once the chain is cycling: `gary`
    /// stands for one chain tick every `code` kanu steps (8 T each), and code 0
    /// pins the prescaler terminal so every chain tick passes.
    pub(super) fn increment_cadence(divisor_code: u8) -> u32 {
        if divisor_code == 0 {
            4
        } else {
            8 * divisor_code as u32
        }
    }
}

/// Divider increments from `divider` to the one whose tap edge rises, given the
/// tapped level standing now.
pub(super) fn increments_to_tap_rise(divider: u16, prev_tap: bool, shift: u8) -> u32 {
    let period = 1u32 << (shift + 1);
    let half = 1u32 << shift;
    let next = divider.wrapping_add(1) as u32 & 0x3fff;
    if !prev_tap && (next >> shift) & 1 == 1 {
        return 1;
    }
    let ahead = (half + period - (next & (period - 1))) % period;
    if ahead == 0 { 1 + period } else { 1 + ahead }
}

/// Ticks to the first LFSR shift that flips bit 0 while CH4 reaches the mix.
/// Shifts that leave bit 0 alone are inaudible, and every shift is inaudible
/// while the channel is silent — the LFSR is caught up at materialisation.
fn ticks_to_noise_output_change(ch4: &NoiseChannel, entry_phase: u8, bound: u32) -> u32 {
    if ch4.sync_delay > 0 {
        // The whole chain is frozen; nothing to predict past the thaw.
        return ch4.sync_delay as u32;
    }
    if !ch4.enabled.enabled {
        return u32::MAX;
    }
    let shift = ch4.frequency_and_randomness.clock_shift();
    if shift >= 14 {
        // The tapped bit is above the 14-bit divider — it never rises.
        return u32::MAX;
    }
    let mut chain = NoiseChain::new(ch4, entry_phase);
    let Some(first) = first_increment(&mut chain, bound) else {
        return u32::MAX;
    };
    let cadence = NoiseChain::increment_cadence(chain.divisor_code);
    let rise_period = 1u32 << (shift + 1);
    let mut increment = increments_to_tap_rise(ch4.divider, ch4.prev_tap, shift);
    let mut lfsr = ch4.lfsr;
    let mut skip_first = ch4.skip_first_clock;
    let short_mode = ch4.frequency_and_randomness.short_mode();
    loop {
        let offset = first + (increment - 1) * cadence;
        if offset > bound {
            return u32::MAX;
        }
        if skip_first {
            skip_first = false;
        } else if super::channels::noise::shift_lfsr(&mut lfsr, short_mode) {
            return offset - 1;
        }
        increment += rise_period;
    }
}

/// Ticks to the chain's next divider increment, walking the divisor prescaler
/// up to its terminal.
pub(super) fn first_increment(chain: &mut NoiseChain, bound: u32) -> Option<u32> {
    while chain.offset <= bound {
        if chain.step() {
            return Some(chain.offset);
        }
        chain.advance();
    }
    None
}
