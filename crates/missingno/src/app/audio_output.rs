use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// Charge factor of the board's output coupling caps per 4 MiHz T-cycle
// (SameBoy's model constant; ~28 Hz cutoff). The console emits SO1/SO2
// unfiltered — the DC block is board-level, between chip pad and jack.
const DC_BLOCK_CHARGE_PER_TCYCLE: f64 = 0.999958;
const TCYCLES_PER_SAMPLE: f64 = 4_194_304.0 / 44_100.0;

/// First-order DC blocker: `y[n] = x[n] - x[n-1] + R*y[n-1]`.
struct DcBlocker {
    r: f32,
    prev_in: f32,
    prev_out: f32,
}

impl DcBlocker {
    fn new() -> Self {
        Self {
            r: DC_BLOCK_CHARGE_PER_TCYCLE.powf(TCYCLES_PER_SAMPLE) as f32,
            prev_in: 0.0,
            prev_out: 0.0,
        }
    }

    fn filter(&mut self, x: f32) -> f32 {
        let y = x - self.prev_in + self.r * self.prev_out;
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}

pub struct AudioOutput {
    _stream: cpal::Stream,
    producer: rtrb::Producer<(f32, f32)>,
    dc_block_left: DcBlocker,
    dc_block_right: DcBlocker,
}

impl AudioOutput {
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: 44100,
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
            dc_block_left: DcBlocker::new(),
            dc_block_right: DcBlocker::new(),
        })
    }

    pub fn push_samples(&mut self, samples: &[(f32, f32)]) {
        for &(left, right) in samples {
            let filtered = (
                self.dc_block_left.filter(left),
                self.dc_block_right.filter(right),
            );
            let _ = self.producer.push(filtered);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DcBlocker;

    #[test]
    fn dc_blocker_removes_constant_offset() {
        let mut f = DcBlocker::new();
        let mut y = 0.0;
        for _ in 0..44_100 {
            y = f.filter(0.5);
        }
        assert!(
            y.abs() < 0.001,
            "constant input should decay to ~0, got {y}"
        );
    }

    #[test]
    fn dc_blocker_passes_step_transient() {
        let mut f = DcBlocker::new();
        for _ in 0..1_000 {
            f.filter(0.0);
        }
        let y = f.filter(0.25);
        assert!(
            (y - 0.25).abs() < 0.002,
            "step edge should pass at full height, got {y}"
        );
    }
}
