//! Analog stages a board puts between a chip's pads and its jack.
//!
//! A board network is stated as its components — the values printed on the
//! schematic — because those are checkable against the drawing, and because a
//! filter coefficient only exists once a sample rate is chosen. A core states
//! the circuit; the consumer instantiates it at whatever rate it runs.

use std::f32::consts::TAU;

/// A series capacitor into a resistance to ground: the AC coupling a board
/// puts between an output pad and the stage after it. It blocks the pad's
/// resting DC and passes the signal above its corner.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RcHighPass {
    pub resistance_ohms: f32,
    pub capacitance_farads: f32,
}

impl RcHighPass {
    /// The −3 dB corner.
    fn cutoff_hz(self) -> f32 {
        1.0 / (TAU * self.resistance_ohms * self.capacitance_farads)
    }

    pub fn high_pass(self) -> HighPass {
        HighPass {
            cutoff_hz: self.cutoff_hz(),
        }
    }
}

/// A coupling's corner, however it was arrived at. Boards whose components are
/// known state them as an [`RcHighPass`] and derive this; boards known only by
/// a fitted decay say so directly, rather than inventing components to match.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HighPass {
    pub cutoff_hz: f32,
}

impl HighPass {
    /// From a per-cycle charge factor: the form a coupling takes when it was
    /// fitted against a running console rather than read off a schematic.
    pub fn from_decay_per_cycle(decay: f32, cycle_rate_hz: f32) -> Self {
        HighPass {
            cutoff_hz: -decay.ln() * cycle_rate_hz / TAU,
        }
    }

    pub fn at_sample_rate(self, sample_rate_hz: f32) -> OnePoleHighPass {
        OnePoleHighPass::from_cutoff(self.cutoff_hz, sample_rate_hz)
    }
}

/// A one-pole high-pass: `y = x - x₋₁ + pole·y₋₁`, the discrete form of an RC
/// coupling at a fixed sample rate.
#[derive(Clone, Copy, Debug)]
pub struct OnePoleHighPass {
    pole: f32,
    previous_input: f32,
    previous_output: f32,
}

impl OnePoleHighPass {
    fn from_cutoff(cutoff_hz: f32, sample_rate_hz: f32) -> Self {
        Self::from_pole((-TAU * cutoff_hz / sample_rate_hz).exp())
    }

    /// For a coupling known only by its pole — a measured or fitted decay whose
    /// components have never been traced to a schematic.
    fn from_pole(pole: f32) -> Self {
        OnePoleHighPass {
            pole,
            previous_input: 0.0,
            previous_output: 0.0,
        }
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let output = input - self.previous_input + self.pole * self.previous_output;
        self.previous_input = input;
        self.previous_output = output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Atari 2600's audio coupling: 0.1 µF into 18K.
    const ATARI_COUPLING: RcHighPass = RcHighPass {
        resistance_ohms: 18_000.0,
        capacitance_farads: 100e-9,
    };

    #[test]
    fn components_give_the_corner() {
        // 1/(2π·18k·0.1µ) ≈ 88.4 Hz.
        assert!((ATARI_COUPLING.cutoff_hz() - 88.42).abs() < 0.01);
    }

    #[test]
    fn a_coupling_blocks_dc() {
        // A held level charges the cap and stops reaching the far side.
        let mut filter = ATARI_COUPLING.high_pass().at_sample_rate(44_100.0);
        let mut output = 0.0;
        for _ in 0..44_100 {
            output = filter.process(1.0);
        }
        assert!(output.abs() < 1e-3, "DC survived: {output}");
    }

    #[test]
    fn a_coupling_passes_the_band() {
        // 1 kHz sits far above the corner, so it comes through near unity.
        let mut filter = ATARI_COUPLING.high_pass().at_sample_rate(44_100.0);
        let mut peak: f32 = 0.0;
        for n in 0..441 {
            let phase = TAU * 1_000.0 * n as f32 / 44_100.0;
            peak = peak.max(filter.process(phase.sin()).abs());
        }
        assert!(peak > 0.99, "band-pass attenuated to {peak}");
    }

    #[test]
    fn a_fitted_decay_reaches_the_same_filter_as_its_corner() {
        // A per-cycle charge factor and the corner it implies are the same
        // coupling: stating either must land on the same pole.
        let decay = 0.999958f32;
        let coupling = HighPass::from_decay_per_cycle(decay, 4_194_304.0);
        let mut from_corner = coupling.at_sample_rate(44_100.0);
        let mut from_decay = OnePoleHighPass::from_pole(decay.powf(4_194_304.0 / 44_100.0));
        for n in 0..256 {
            let x = (n as f32 * 0.05).sin();
            assert!((from_corner.process(x) - from_decay.process(x)).abs() < 1e-5);
        }
    }

    #[test]
    fn the_pole_route_matches_the_component_route() {
        let from_components = ATARI_COUPLING.high_pass().at_sample_rate(44_100.0);
        let pole = (-TAU * 88.4194 / 44_100.0).exp();
        let mut a = from_components;
        let mut b = OnePoleHighPass::from_pole(pole);
        for n in 0..64 {
            let x = (n as f32 * 0.1).sin();
            assert!((a.process(x) - b.process(x)).abs() < 1e-5);
        }
    }
}
