//! Exact reconstruction of a skipped span.
//!
//! Every field the slow path would have written across the skipped ticks is
//! recomputed here in closed form. The span predictor only arms over states
//! these forms cover, so "as if ticked" is an identity, not an approximation.

use super::channels::noise::{NoiseChannel, shift_lfsr};
use super::channels::pulse::PulseChannel;
use super::channels::sweep::{Sweep, SweepUnit};
use super::channels::wave::WaveChannel;
use super::span::{
    CLOCK_RISE_PHASE, NoiseChain, SWEEP_STEP_PHASE, increments_to_tap_rise, phase_count,
};
use super::{ApuSpec, Audio, DIV_APU_BIT, DIV_APU_BIT_DOUBLE};

/// Where the shared prescaler stands `ticks` calls on. It advances once per
/// call in both regimes and neither `channel_clock` nor CH4's `mhz_prescaler`
/// is touched while a span is in flight.
fn prescaler_phase(counter: u8, ticks: u32) -> u8 {
    ((counter as u32 + ticks) & 0b11) as u8
}

impl<A: ApuSpec> Audio<A> {
    /// Advance the whole APU across the ticks the fast path skipped.
    #[cold]
    pub(super) fn materialize_span(&mut self) {
        let (ticks, last_div_counter, double_speed) = self.span.take_skipped();
        if ticks == 0 {
            return;
        }
        debug_assert_eq!(
            self.channel_clock.counter, self.channels.ch4.mhz_prescaler.counter,
            "the channel clock and CH4 prescaler share one phase"
        );
        let entry_phase = prescaler_phase(self.channel_clock.counter, 1);
        // CALO↑ (chN_1mhz) and AJER↑ (the sweep-step tap) across the span.
        let channel_clock_rises = phase_count(entry_phase, CLOCK_RISE_PHASE, ticks);
        let sweep_clock_rises = phase_count(entry_phase, SWEEP_STEP_PHASE, ticks);

        advance_pulse_sweep(
            &mut self.channels.ch1,
            channel_clock_rises,
            sweep_clock_rises,
        );
        advance_pulse(&mut self.channels.ch2, channel_clock_rises);
        advance_wave(&mut self.channels.ch3, ticks);
        let noise_flip_at = advance_noise(
            &mut self.channels.ch4,
            ticks,
            entry_phase,
            channel_clock_rises,
        );

        self.channel_clock.counter = prescaler_phase(self.channel_clock.counter, ticks);
        let tap = if A::DOUBLE_SPEED && double_speed {
            DIV_APU_BIT_DOUBLE
        } else {
            DIV_APU_BIT
        };
        self.prev_div_apu_bit = last_div_counter & tap != 0;
        self.advance_mixer(ticks, noise_flip_at);
    }

    /// The run-length-compressed mix and the host sample windows. `last_mix` is
    /// constant across the span by construction, so each window's fold is one
    /// integer product; the only flush inside the span is the one a silent
    /// CH4's LFSR provokes, which changes nothing but the run partition.
    fn advance_mixer(&mut self, ticks: u32, noise_flip_at: Option<u32>) {
        let dac_codes = self.channels.dac_codes();
        let mut consumed = 0;
        let mut window = 1u64;
        loop {
            // The window closes on the first tick whose count reaches it.
            let close = self.sample_clock.ticks_until(window);
            if close > ticks as u64 {
                break;
            }
            let close = close as u32;
            self.mix_run += close - consumed;
            consumed = close;
            self.fold_pending();
            let count = self.sample_accum_count as f32;
            self.sample_buffer.push((
                self.sample_accum_left / count,
                self.sample_accum_right / count,
            ));
            self.sample_accum_left = 0.0;
            self.sample_accum_right = 0.0;
            self.sample_accum_count = 0;
            if let Some(rings) = &mut self.wave_capture {
                for (ring, code) in rings.iter_mut().zip(dac_codes) {
                    ring.push(code);
                }
            }
            window += 1;
        }
        let closed = self.sample_clock.advance(ticks as u64);
        debug_assert_eq!(closed, window - 1, "a window closed outside the walk");

        match noise_flip_at.filter(|&at| at > consumed) {
            Some(at) => {
                self.mix_run += at - 1 - consumed;
                self.flush_mix_run();
                self.mix_run = ticks - at + 1;
            }
            None => self.mix_run += ticks - consumed,
        }
    }
}

/// CH1: the sweep hold and adder counter run whatever the channel's state; the
/// divider only runs while it is enabled (and so contributing — a DAC-off write
/// disables the channel).
fn advance_pulse_sweep(
    ch1: &mut PulseChannel<Sweep>,
    channel_clock_rises: u32,
    sweep_clock_rises: u32,
) {
    ch1.sweep.load_hold = ch1
        .sweep
        .load_hold
        .saturating_sub(channel_clock_rises.min(255) as u8);
    if ch1.sweep.calc_steps > 0 {
        // The predictor ends the span before the counter saturates.
        ch1.sweep.calc_steps -= sweep_clock_rises as u8;
        debug_assert!(ch1.sweep.calc_steps > 0);
    }
    advance_pulse(ch1, channel_clock_rises);
}

fn advance_pulse<S: SweepUnit>(ch: &mut PulseChannel<S>, channel_clock_rises: u32) {
    if !ch.enabled.enabled {
        return;
    }
    let duty: [bool; 8] = std::array::from_fn(|step| ch.duty_bit(step as u8));
    let latch = advance_period_divider(
        &mut ch.divider.counter,
        &mut ch.wave_duty_position,
        &mut ch.overflow_pulse,
        ch.period.0,
        channel_clock_rises,
        &duty,
    );
    debug_assert!(latch.is_none_or(|bit| bit == ch.pwm_latch));
}

/// The shared 11-bit period divider: counts to 0x7FF, reloads from `period`
/// and captures the duty step one clock before it advances. Returns the last
/// bit `duwo`/`dome` captured, which the span guarantees is the one already
/// latched.
fn advance_period_divider(
    counter: &mut u16,
    position: &mut u8,
    frst: &mut bool,
    period: u16,
    clocks: u32,
    duty: &[bool; 8],
) -> Option<bool> {
    let period = period & 0x7FF;
    let mut count = *counter as u32 & 0x7FF;
    let mut remaining = clocks;
    let mut latched = None;
    while remaining > 0 {
        let to_overflow = if count >= 0x7FF { 1 } else { 0x800 - count };
        if *frst {
            *position = (*position + 1) % 8;
            *frst = false;
        }
        if to_overflow > remaining {
            count += remaining;
            break;
        }
        latched = Some(duty[*position as usize]);
        *frst = true;
        count = period as u32;
        remaining -= to_overflow;
    }
    *counter = count as u16;
    latched
}

/// CH3: `cery` halves the dot clock and the divider counts on its rise. The
/// span ends before any overflow, so the whole load/latch chain stays quiet and
/// the wave position holds.
fn advance_wave(ch3: &mut WaveChannel, ticks: u32) {
    let rises = if ch3.ch3_2mhz {
        ticks / 2
    } else {
        ticks.div_ceil(2)
    };
    ch3.ch3_2mhz ^= ticks % 2 == 1;
    if ch3.ch3_fdis || ch3.frequency_timer == 0 {
        return;
    }
    ch3.frequency_timer -= rises as u16;
    debug_assert!(ch3.frequency_timer > 0);
}

/// CH4: the divisor prescaler, the 14-bit divider and the LFSR, which free-run
/// whenever the APU is powered. Returns the tick offset of the last LFSR shift
/// that flipped bit 0 — inaudible (the channel is silent, or the span would
/// have ended), but it still repartitions the mix run.
fn advance_noise(
    ch4: &mut NoiseChannel,
    ticks: u32,
    entry_phase: u8,
    channel_clock_rises: u32,
) -> Option<u32> {
    ch4.mhz_prescaler.counter = prescaler_phase(ch4.mhz_prescaler.counter, ticks);
    if ch4.sync_delay > 0 {
        // The chain is frozen for the whole span; only the hold counts down.
        ch4.prescaler_512khz ^= channel_clock_rises % 2 == 1;
        ch4.sync_delay -= ticks as u16;
        return None;
    }
    let mut chain = NoiseChain::new(ch4, entry_phase);
    let cadence = NoiseChain::increment_cadence(chain.divisor_code);
    let mut last_chain_tick = None;
    let mut first_increment = None;
    let mut increments = 0;
    // Up to the first increment the divisor prescaler has to be walked; from
    // there the chain is canonical (terminal at every increment), so the whole
    // cadences are a jump and only the tail past the last one is walked.
    while chain.offset <= ticks {
        let passed = chain.step();
        last_chain_tick = Some(chain.offset);
        if passed {
            increments = 1 + (ticks - chain.offset) / cadence;
            let jumped = (increments - 1) * cadence;
            first_increment = Some(chain.offset);
            chain.offset += jumped;
            chain.prescaler_512khz ^= (jumped / 4) % 2 == 1;
            last_chain_tick = Some(chain.offset);
            chain.advance();
            break;
        }
        chain.advance();
    }
    while chain.offset <= ticks {
        let passed = chain.step();
        debug_assert!(!passed, "a second increment landed inside the jumped span");
        last_chain_tick = Some(chain.offset);
        chain.advance();
    }
    ch4.prescaler = chain.prescaler;
    ch4.prescaler_512khz ^= channel_clock_rises % 2 == 1;
    match last_chain_tick {
        Some(at) => {
            ch4.divider_clock_enabled = chain.prescaler == 0b111;
            ch4.divider_subcounter = 4 - (ticks - at) as u16;
        }
        None => ch4.divider_subcounter -= ticks as u16,
    }
    let first_increment = first_increment?;

    let shift = ch4.frequency_and_randomness.clock_shift();
    let start = ch4.divider;
    let prev_tap = ch4.prev_tap;
    ch4.divider = ch4.divider.wrapping_add(increments as u16) & 0x3fff;
    if shift >= 14 {
        // The tapped bit is above the 14-bit divider: it never rises.
        ch4.prev_tap = false;
        return None;
    }
    ch4.prev_tap = (ch4.divider >> shift) & 1 != 0;
    let first_rise = increments_to_tap_rise(start, prev_tap, shift);
    if first_rise > increments {
        return None;
    }
    let rise_period = 1u32 << (shift + 1);
    let rises = 1 + (increments - first_rise) / rise_period;
    let short_mode = ch4.frequency_and_randomness.short_mode();
    let mut flip_at = None;
    for rise in 0..rises {
        if ch4.skip_first_clock {
            ch4.skip_first_clock = false;
            continue;
        }
        if shift_lfsr(&mut ch4.lfsr, short_mode) {
            flip_at = Some(first_increment + (first_rise - 1 + rise * rise_period) * cadence);
        }
    }
    flip_at
}
