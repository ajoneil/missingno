use super::*;

fn discrete() -> Psg {
    Psg::new(Variant::DiscreteTi)
}

fn sega() -> Psg {
    Psg::new(Variant::SegaIntegrated)
}

fn tone0(psg: &Psg) -> bool {
    psg.tones[0].output
}

fn noise_flip_flop(psg: &Psg) -> bool {
    psg.noise.output
}

fn shift_register(psg: &Psg) -> u16 {
    psg.noise.lfsr
}

fn tick(psg: &mut Psg, clocks: u32) {
    for _ in 0..clocks {
        psg.tick();
    }
}

/// Input clocks until `probe` first differs from its value now.
fn clocks_until_change<T: Copy + PartialEq + std::fmt::Debug>(
    psg: &mut Psg,
    probe: fn(&Psg) -> T,
    limit: u32,
) -> u32 {
    let before = probe(psg);
    for clock in 1..=limit {
        psg.tick();
        if probe(psg) != before {
            return clock;
        }
    }
    panic!("{before:?} unchanged after {limit} input clocks");
}

/// The bits leaving the shift register, one per shift.
fn shift_sequence(variant: Variant, mode: NoiseMode, shifts: usize) -> Vec<u8> {
    let mut noise = NoiseChannel::new(variant);
    noise.mode = mode;
    (0..shifts)
        .map(|_| {
            let out = (noise.lfsr & 1) as u8;
            noise.shift(variant);
            out
        })
        .collect()
}

/// Shifts before the register returns to its cleared state — which, the
/// state deciding the whole future, is the output sequence's period.
fn shift_cycle_length(variant: Variant, mode: NoiseMode) -> usize {
    let mut noise = NoiseChannel::new(variant);
    noise.mode = mode;
    let cleared = noise.lfsr;
    for shift in 1..=1 << 17 {
        noise.shift(variant);
        if noise.lfsr == cleared {
            return shift;
        }
    }
    panic!("the shift register never returned to its cleared state");
}

#[test]
fn an_addressing_byte_lands_its_nibble_immediately() {
    let mut psg = discrete();
    psg.write(0x8F);
    assert_eq!(psg.tone_periods()[0], 0x00F);
}

#[test]
fn a_data_byte_fills_a_tone_registers_high_six_bits() {
    let mut psg = discrete();
    psg.write(0x8E);
    psg.write(0x0F);
    assert_eq!(psg.tone_periods()[0], 0x0FE);
}

#[test]
fn consecutive_bytes_walk_the_tone_register() {
    // Each byte lands as it arrives; neither waits for the other.
    let mut psg = discrete();
    psg.write(0x80);
    assert_eq!(psg.tone_periods()[0] & 0x00F, 0x000);
    psg.write(0x00);
    assert_eq!(psg.tone_periods()[0], 0x000);
    psg.write(0x8F);
    assert_eq!(psg.tone_periods()[0], 0x00F);
    psg.write(0x3F);
    assert_eq!(psg.tone_periods()[0], 0x3FF);
}

#[test]
fn a_data_byte_after_an_attenuation_address_updates_the_volume() {
    let mut psg = discrete();
    psg.write(0xDF);
    psg.write(0x00);
    assert_eq!(psg.attenuations()[2], 0x00);
}

#[test]
fn a_data_byte_after_a_noise_address_updates_the_control() {
    let mut psg = discrete();
    psg.write(0xE5);
    assert_eq!(psg.noise_control(), 0x05);
    psg.write(0x04);
    assert_eq!(psg.noise_control(), 0x04);
}

#[test]
fn the_noise_register_discards_the_nibbles_high_bit() {
    let mut psg = discrete();
    psg.write(0xEF);
    assert_eq!(psg.noise_control(), 0x07);
}

#[test]
fn the_latched_register_survives_data_bytes() {
    let mut psg = discrete();
    psg.write(0x8E);
    psg.write(0x0F);
    psg.write(0x00);
    assert_eq!(psg.tone_periods()[0], 0x00E);
}

#[test]
fn a_tone_toggles_every_sixteen_input_clocks_per_period_step() {
    let mut psg = discrete();
    psg.write(0x83);
    psg.write(0x00);
    clocks_until_change(&mut psg, tone0, 64);
    for _ in 0..4 {
        assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 3);
    }
}

#[test]
fn a_period_rewrite_applies_at_the_next_borrow() {
    let mut psg = discrete();
    psg.write(0x88);
    psg.write(0x00);
    clocks_until_change(&mut psg, tone0, 64);
    tick(&mut psg, 16 * 3);
    psg.write(0x82);
    psg.write(0x00);
    // The counter keeps the five internal clocks it still owes.
    assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 5);
    assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 2);
}

#[test]
fn a_zero_period_counts_the_full_span_on_the_discrete_part() {
    let mut psg = discrete();
    clocks_until_change(&mut psg, tone0, 64);
    assert_eq!(
        clocks_until_change(&mut psg, tone0, 32_768),
        16 * FULL_TONE_SPAN as u32
    );
}

#[test]
fn a_zero_period_holds_the_channel_on_the_integrated_part() {
    let mut psg = sega();
    let held = tone0(&psg);
    tick(&mut psg, 2 * 16 * FULL_TONE_SPAN as u32);
    assert_eq!(tone0(&psg), held);
}

#[test]
fn the_fixed_shift_rates_divide_the_input_clock() {
    for (control, input_clocks) in [(0xE0u8, 512u32), (0xE1, 1024), (0xE2, 2048)] {
        let mut psg = discrete();
        psg.write(control);
        clocks_until_change(&mut psg, shift_register, 4096);
        assert_eq!(
            clocks_until_change(&mut psg, shift_register, 8192),
            input_clocks
        );
    }
}

#[test]
fn the_follow_rate_tracks_the_third_tone_register() {
    let mut psg = discrete();
    psg.write(0xC4);
    psg.write(0x00);
    psg.write(0xE3);
    clocks_until_change(&mut psg, shift_register, 1024);
    assert_eq!(
        clocks_until_change(&mut psg, shift_register, 1024),
        2 * 16 * 4
    );
    psg.write(0xC8);
    psg.write(0x00);
    clocks_until_change(&mut psg, shift_register, 2048);
    assert_eq!(
        clocks_until_change(&mut psg, shift_register, 2048),
        2 * 16 * 8
    );
}

#[test]
fn the_shift_register_advances_on_rising_edges_only() {
    let mut psg = discrete();
    psg.write(0xE0);
    let (mut toggles, mut shifts) = (0, 0);
    let mut output = noise_flip_flop(&psg);
    let mut register = shift_register(&psg);
    for _ in 0..16 * 16 * 21 {
        psg.tick();
        let toggled = noise_flip_flop(&psg) != output;
        if shift_register(&psg) != register {
            assert!(toggled && noise_flip_flop(&psg));
            register = shift_register(&psg);
            shifts += 1;
        }
        if toggled {
            output = !output;
            toggles += 1;
        }
    }
    assert_eq!(toggles, 21);
    assert_eq!(shifts, 10);
}

#[test]
fn the_discrete_white_sequence_runs_32767_shifts() {
    assert_eq!(
        shift_cycle_length(Variant::DiscreteTi, NoiseMode::White),
        32767
    );
    // Taps at bits 0 and 1 of a 15-bit register.
    let sequence = shift_sequence(Variant::DiscreteTi, NoiseMode::White, 200);
    for i in 15..sequence.len() {
        assert_eq!(sequence[i], sequence[i - 15] ^ sequence[i - 14]);
    }
}

#[test]
fn the_integrated_white_sequence_runs_57337_shifts() {
    assert_eq!(
        shift_cycle_length(Variant::SegaIntegrated, NoiseMode::White),
        57337
    );
    // Taps at bits 0 and 3 of a 16-bit register.
    let sequence = shift_sequence(Variant::SegaIntegrated, NoiseMode::White, 200);
    for i in 16..sequence.len() {
        assert_eq!(sequence[i], sequence[i - 16] ^ sequence[i - 13]);
    }
}

#[test]
fn periodic_noise_is_a_one_in_fifteen_duty_on_the_discrete_part() {
    assert_eq!(
        shift_cycle_length(Variant::DiscreteTi, NoiseMode::Periodic),
        15
    );
    let sequence = shift_sequence(Variant::DiscreteTi, NoiseMode::Periodic, 150);
    assert_eq!(sequence.iter().filter(|&&bit| bit == 1).count(), 10);
}

#[test]
fn periodic_noise_is_a_one_in_sixteen_duty_on_the_integrated_part() {
    assert_eq!(
        shift_cycle_length(Variant::SegaIntegrated, NoiseMode::Periodic),
        16
    );
    let sequence = shift_sequence(Variant::SegaIntegrated, NoiseMode::Periodic, 160);
    assert_eq!(sequence.iter().filter(|&&bit| bit == 1).count(), 10);
}

#[test]
fn a_noise_control_write_clears_the_shift_register() {
    let cleared = Variant::DiscreteTi.lfsr_shift_in();
    let mut psg = discrete();
    psg.write(0xE4);
    tick(&mut psg, 16 * 16 * 40);
    assert_ne!(shift_register(&psg), cleared);
    psg.write(0xE4);
    assert_eq!(shift_register(&psg), cleared);
}

#[test]
fn each_attenuation_step_is_two_decibels() {
    for step in 1..MUTE_ATTENUATION {
        let ratio = amplitude(step) / amplitude(step - 1);
        assert!((20.0 * ratio.log10() + DECIBELS_PER_STEP).abs() < 1e-4);
    }
    assert_eq!(amplitude(MUTE_ATTENUATION), 0.0);
}

#[test]
fn the_summing_stage_is_linear() {
    let mut psg = discrete();
    for channel in 0..CHANNELS as u8 {
        psg.write(0x9F | (channel << 5));
    }
    assert_eq!(psg.level(), 0.0);
    psg.write(0x90);
    let one = psg.level();
    psg.write(0xB0);
    assert!((psg.level() - 2.0 * one).abs() < 1e-6);
}

#[test]
fn a_muted_channel_codes_zero() {
    let mut psg = discrete();
    psg.write(0x9F);
    assert!(tone0(&psg));
    assert_eq!(psg.dac_codes()[0], 0);
}

#[test]
fn an_unattenuated_conducting_channel_codes_full_scale() {
    let psg = discrete();
    assert_eq!(psg.attenuations()[0], 0);
    assert!(tone0(&psg));
    assert_eq!(psg.dac_codes()[0], 15);
}

#[test]
fn a_tone_code_follows_its_flip_flop() {
    let mut psg = discrete();
    psg.write(0x83);
    psg.write(0x00);
    assert!(tone0(&psg));
    assert_eq!(psg.dac_codes()[0], 15);
    clocks_until_change(&mut psg, tone0, 64);
    assert_eq!(psg.dac_codes()[0], 0);
    clocks_until_change(&mut psg, tone0, 64);
    assert_eq!(psg.dac_codes()[0], 15);
}

#[test]
fn the_noise_code_follows_the_bit_leaving_the_shift_register() {
    let mut psg = discrete();
    psg.write(0xE4);
    let (mut set, mut cleared) = (false, false);
    for _ in 0..16 * 512 * 8 {
        psg.tick();
        let conducting = shift_register(&psg) & 1 != 0;
        assert_eq!(
            psg.dac_codes()[Channel::Noise.index()],
            if conducting { 15 } else { 0 }
        );
        set |= conducting;
        cleared |= !conducting;
    }
    assert!(set && cleared, "the output bit never took both values");
}

#[test]
fn ready_returns_thirty_two_input_clocks_after_a_write() {
    let mut psg = discrete();
    assert!(psg.ready());
    psg.write(0x9F);
    for _ in 0..READY_LOW_CLOCKS - 1 {
        psg.tick();
        assert!(!psg.ready());
    }
    psg.tick();
    assert!(psg.ready());
}

#[test]
fn the_integrated_part_is_always_ready() {
    let mut psg = sega();
    psg.write(0x9F);
    assert!(psg.ready());
}

#[test]
fn the_discrete_part_powers_on_sounding() {
    let psg = discrete();
    assert_eq!(psg.tone_periods(), [0; TONE_CHANNELS]);
    assert_eq!(psg.attenuations(), [0; CHANNELS]);
    assert!(psg.level() > 0.0);
}

#[test]
fn the_discrete_part_powers_on_addressing_the_first_tone() {
    let mut psg = discrete();
    psg.write(0x3F);
    assert_eq!(psg.tone_periods()[0], 0x3F0);
}

#[test]
fn the_integrated_part_powers_on_silent() {
    let psg = sega();
    assert_eq!(psg.attenuations(), [MUTE_ATTENUATION; CHANNELS]);
    assert_eq!(psg.level(), 0.0);
}

#[test]
fn the_integrated_part_powers_on_addressing_the_second_attenuation() {
    let mut psg = sega();
    psg.write(0x00);
    assert_eq!(psg.attenuations()[1], 0x00);
}

/// The captured state names everything the generators run on, so a restored
/// part stays in step clock for clock.
#[test]
fn a_captured_state_survives_its_own_restore() {
    let mut psg = discrete();
    psg.write(0x8A); // tone 1 period low
    psg.write(0x0C); // ...and its high bits
    psg.write(0x92); // tone 1 attenuation
    psg.write(0xE5); // noise: white, ÷1024
    tick(&mut psg, 900);

    let state = psg.boundary_state();
    let mut restored = discrete();
    restored.restore_boundary(&state);
    assert_eq!(restored.boundary_state(), state);

    for _ in 0..64 {
        tick(&mut psg, 17);
        tick(&mut restored, 17);
        assert_eq!(restored.boundary_state(), psg.boundary_state());
        assert_eq!(restored.dac_codes(), psg.dac_codes());
    }
}

/// READY's countdown rides the state, so a restore taken inside a byte load
/// still stalls the board for what is left of it.
#[test]
fn a_restored_part_finishes_the_byte_it_was_loading() {
    let mut psg = discrete();
    psg.write(0x9F);
    tick(&mut psg, 8);
    assert!(!psg.ready());

    let mut restored = discrete();
    restored.restore_boundary(&psg.boundary_state());
    assert!(!restored.ready());
    tick(&mut restored, READY_LOW_CLOCKS as u32 - 8);
    assert!(restored.ready());
}
