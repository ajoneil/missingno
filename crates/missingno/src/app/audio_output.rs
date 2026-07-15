use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use missingno_hw::{HighPass, OnePoleHighPass};

const SAMPLE_RATE: u32 = 44_100;

/// The device end of the audio path. It plays what a console's board delivers
/// to the jack, so the only stage here is that board's coupling — which the
/// console states and this retunes to whenever the machine changes.
pub struct AudioOutput {
    _stream: cpal::Stream,
    producer: rtrb::Producer<(f32, f32)>,
    coupling: Option<Coupling>,
}

struct Coupling {
    spec: HighPass,
    left: OnePoleHighPass,
    right: OnePoleHighPass,
}

impl AudioOutput {
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };

        let (producer, mut consumer) = rtrb::RingBuffer::new(4096);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_exact_mut(2) {
                        let (left, right) = consumer.pop().unwrap_or((0.0, 0.0));
                        frame[0] = left;
                        frame[1] = right;
                    }
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            )
            .ok()?;

        stream.play().ok()?;

        Some(Self {
            _stream: stream,
            producer,
            coupling: None,
        })
    }

    /// Play a console's samples through the coupling its board provides. A
    /// board the console does not describe leaves the samples untouched.
    pub fn push_samples(&mut self, samples: &[(f32, f32)], coupling: Option<HighPass>) {
        self.tune(coupling);
        for &(left, right) in samples {
            let played = match &mut self.coupling {
                Some(coupling) => (coupling.left.process(left), coupling.right.process(right)),
                None => (left, right),
            };
            let _ = self.producer.push(played);
        }
    }

    /// Rebuild the filters when the machine on the other end changes; holding
    /// them steady otherwise keeps each one's charge across the stream.
    fn tune(&mut self, spec: Option<HighPass>) {
        if self.coupling.as_ref().map(|coupling| coupling.spec) == spec {
            return;
        }
        self.coupling = spec.map(|spec| Coupling {
            spec,
            left: spec.at_sample_rate(SAMPLE_RATE as f32),
            right: spec.at_sample_rate(SAMPLE_RATE as f32),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_boy_coupling() -> OnePoleHighPass {
        missingno_gb::board::audio_coupling().at_sample_rate(SAMPLE_RATE as f32)
    }

    #[test]
    fn a_coupling_removes_a_constant_offset() {
        let mut f = game_boy_coupling();
        let mut y = 0.0f32;
        for _ in 0..44_100 {
            y = f.process(0.5);
        }
        assert!(
            y.abs() < 0.001,
            "constant input should decay to ~0, got {y}"
        );
    }

    #[test]
    fn a_coupling_passes_a_step_transient() {
        let mut f = game_boy_coupling();
        for _ in 0..1_000 {
            f.process(0.0);
        }
        let y: f32 = f.process(0.25);
        assert!(
            (y - 0.25).abs() < 0.002,
            "step edge should pass at full height, got {y}"
        );
    }

    #[test]
    fn the_consoles_do_not_share_a_coupling() {
        // The Atari's board is drawn: 0.1 µF into 18K. The Game Boy's is a
        // fitted decay. They are different boards and must not sound alike.
        let atari = missingno_vcs::board::AUDIO_COUPLING.high_pass();
        let game_boy = missingno_gb::board::audio_coupling();
        assert!(atari.cutoff_hz > game_boy.cutoff_hz * 2.0);
    }
}
