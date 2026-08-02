//! Exact reconstruction of a skipped span.
//!
//! Every field the slow path would have written across the skipped ticks is
//! recomputed here in closed form. The span predictor only arms over states
//! these forms cover, so "as if ticked" is an identity, not an approximation.

use super::channels::noise::{NoiseChannel, shift_lfsr};
use super::channels::pulse::PulseChannel;
use super::channels::pulse_sweep::PulseSweepChannel;
use super::channels::wave::WaveChannel;
use super::span::{NoiseChain, increments_to_tap_rise, t_index_count};
use super::{ApuSpec, Audio, DIV_APU_BIT, T_CYCLES_PER_SAMPLE};

impl<A: ApuSpec> Audio<A> {
    /// Advance the whole APU across the ticks the fast path skipped.
    #[cold]
    pub(super) fn materialize_span(&mut self) {
        let (ticks, entry_t_index, last_div_counter, last_t_index) = self.span.take_skipped();
        if ticks == 0 {
            return;
        }
        let calo_rises = t_index_count(entry_t_index, 1, ticks);
        let clock_phase_ones = t_index_count(entry_t_index, 0, ticks);

        advance_pulse_sweep(&mut self.channels.ch1, calo_rises, clock_phase_ones);
        advance_pulse(&mut self.channels.ch2, calo_rises);
        advance_wave(&mut self.channels.ch3, ticks);
        let noise_dirty = advance_noise(
            &mut self.channels.ch4,
            ticks,
            entry_t_index,
            last_t_index,
            calo_rises,
        );

        self.channel_clock.counter = (last_t_index + 1) & 0b11;
        self.prev_div_apu_bit = last_div_counter & DIV_APU_BIT != 0;
        self.advance_mixer(ticks, noise_dirty);
    }

    /// The run-length-compressed mix and the host sample windows. `last_mix` is
    /// constant across the span by construction, so each window's fold is one
    /// integer product; the only flush inside the span is the one a silent
    /// CH4's LFSR provokes, which changes nothing but the run partition.
    fn advance_mixer(&mut self, ticks: u32, noise_dirty: Option<u32>) {
        let period = T_CYCLES_PER_SAMPLE as f64;
        let counter = self.sample_counter as f64;
        let dac_codes = self.channels.dac_codes();
        let mut consumed = 0;
        let mut window = 1u32;
        loop {
            // The window closes on the first tick whose counter reaches it.
            let close = (window as f64 * period - counter).ceil() as u32;
            if close > ticks {
                break;
            }
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
        self.sample_counter = (counter + ticks as f64 - (window - 1) as f64 * period) as f32;

        match noise_dirty.filter(|&at| at > consumed) {
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
fn advance_pulse_sweep(ch1: &mut PulseSweepChannel, calo_rises: u32, clock_phase_ones: u32) {
    ch1.sweep_load_hold = ch1
        .sweep_load_hold
        .saturating_sub(calo_rises.min(255) as u8);
    if ch1.sweep_calc_steps > 0 {
        // The predictor ends the span before the counter saturates.
        ch1.sweep_calc_steps -= clock_phase_ones as u8;
        debug_assert!(ch1.sweep_calc_steps > 0);
    }
    if !ch1.enabled.enabled {
        return;
    }
    let duty: [bool; 8] = std::array::from_fn(|step| ch1.duty_bit(step as u8));
    let latch = advance_period_divider(
        &mut ch1.divider.counter,
        &mut ch1.wave_duty_position,
        &mut ch1.ch1_frst,
        ch1.period.0,
        calo_rises,
        &duty,
    );
    debug_assert!(latch.is_none_or(|bit| bit == ch1.pwm_latch));
}

fn advance_pulse(ch2: &mut PulseChannel, calo_rises: u32) {
    if !ch2.enabled.enabled {
        return;
    }
    let duty: [bool; 8] = std::array::from_fn(|step| ch2.duty_bit(step as u8));
    let latch = advance_period_divider(
        &mut ch2.divider.counter,
        &mut ch2.wave_duty_position,
        &mut ch2.ch2_frst,
        ch2.period.0,
        calo_rises,
        &duty,
    );
    debug_assert!(latch.is_none_or(|bit| bit == ch2.pwm_latch));
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
    entry_t_index: u8,
    last_t_index: u8,
    calo_rises: u32,
) -> Option<u32> {
    ch4.mhz_prescaler.counter = (last_t_index + 1) & 0b11;
    if ch4.sync_delay > 0 {
        // The chain is frozen for the whole span; only the hold counts down.
        ch4.jeso ^= calo_rises % 2 == 1;
        ch4.sync_delay -= ticks as u16;
        return None;
    }
    let mut chain = NoiseChain::new(ch4, entry_t_index);
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
            chain.jeso ^= (jumped / 4) % 2 == 1;
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
    ch4.jeso ^= calo_rises % 2 == 1;
    match last_chain_tick {
        Some(at) => {
            ch4.gary = chain.prescaler == 0b111;
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
    let mut dirty_at = None;
    for rise in 0..rises {
        if ch4.skip_first_clock {
            ch4.skip_first_clock = false;
            continue;
        }
        if shift_lfsr(&mut ch4.lfsr, short_mode) {
            dirty_at = Some(first_increment + (first_rise - 1 + rise * rise_period) * cadence);
        }
    }
    dirty_at
}
