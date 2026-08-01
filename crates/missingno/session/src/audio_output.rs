use crate::AudioSink;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use missingno_core::{HighPass, OnePoleHighPass};

const SAMPLE_RATE: u32 = 44_100;

/// The device end of the audio path. The `!Send` cpal [`cpal::Stream`] lives
/// here on the UI thread for the process's life; the session thread that
/// produces samples holds only the [`AudioSink`] half. Dropping this stops the
/// stream — done on the UI thread, so the OS backend never calls the callback
/// into freed memory.
pub struct AudioOutput {
    _stream: cpal::Stream,
}

/// The board coupling applied to a console's samples before they reach the jack.
struct Coupling {
    spec: HighPass,
    left: OnePoleHighPass,
    right: OnePoleHighPass,
}

impl AudioOutput {
    /// Open the default output device, returning the UI-thread stream holder and
    /// the [`AudioSink`] that pushes a console's samples into it through the
    /// board coupling. `None` when no device is available. The sink owns the
    /// ring-buffer producer and the coupling filters, so it moves into the
    /// session thread; the stream stays here.
    pub fn open() -> Option<(Self, AudioSink)> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };

        let (mut producer, mut consumer) = rtrb::RingBuffer::new(4096);

        let stream = device
            .build_output_stream(
                config,
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

        // The sink holds the coupling across calls (a filter keeps its charge),
        // rebuilding only when the console on the other end changes.
        let mut coupling: Option<Coupling> = None;
        let sink: AudioSink = Box::new(move |samples, spec| {
            tune(&mut coupling, spec);
            for (left, right) in samples {
                let played = match &mut coupling {
                    Some(coupling) => (coupling.left.process(left), coupling.right.process(right)),
                    None => (left, right),
                };
                let _ = producer.push(played);
            }
        });

        Some((Self { _stream: stream }, sink))
    }
}

/// Rebuild the coupling filters when the machine changes; holding them steady
/// otherwise keeps each one's charge across the stream.
fn tune(coupling: &mut Option<Coupling>, spec: Option<HighPass>) {
    if coupling.as_ref().map(|coupling| coupling.spec) == spec {
        return;
    }
    *coupling = spec.map(|spec| Coupling {
        spec,
        left: spec.at_sample_rate(SAMPLE_RATE as f32),
        right: spec.at_sample_rate(SAMPLE_RATE as f32),
    });
}

#[cfg(all(test, feature = "gb", feature = "vcs"))]
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
