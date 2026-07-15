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
    pub fn cutoff_hz(self) -> f32 {
        1.0 / (TAU * self.resistance_ohms * self.capacitance_farads)
    }

    pub fn at_sample_rate(self, sample_rate_hz: f32) -> OnePoleHighPass {
        OnePoleHighPass::from_cutoff(self.cutoff_hz(), sample_rate_hz)
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
    pub fn from_cutoff(cutoff_hz: f32, sample_rate_hz: f32) -> Self {
        Self::from_pole((-TAU * cutoff_hz / sample_rate_hz).exp())
    }

    /// For a coupling known only by its pole — a measured or fitted decay whose
    /// components have never been traced to a schematic.
    pub fn from_pole(pole: f32) -> Self {
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
        let mut filter = ATARI_COUPLING.at_sample_rate(44_100.0);
        let mut output = 0.0;
        for _ in 0..44_100 {
            output = filter.process(1.0);
        }
        assert!(output.abs() < 1e-3, "DC survived: {output}");
    }

    #[test]
    fn a_coupling_passes_the_band() {
        // 1 kHz sits far above the corner, so it comes through near unity.
        let mut filter = ATARI_COUPLING.at_sample_rate(44_100.0);
        let mut peak: f32 = 0.0;
        for n in 0..441 {
            let phase = TAU * 1_000.0 * n as f32 / 44_100.0;
            peak = peak.max(filter.process(phase.sin()).abs());
        }
        assert!(peak > 0.99, "band-pass attenuated to {peak}");
    }

    #[test]
    fn the_pole_route_matches_the_component_route() {
        let from_components = ATARI_COUPLING.at_sample_rate(44_100.0);
        let pole = (-TAU * 88.4194 / 44_100.0).exp();
        let mut a = from_components;
        let mut b = OnePoleHighPass::from_pole(pole);
        for n in 0..64 {
            let x = (n as f32 * 0.1).sin();
            assert!((a.process(x) - b.process(x)).abs() < 1e-5);
        }
    }
}
