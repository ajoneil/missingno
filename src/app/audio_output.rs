use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct AudioOutput {
    _stream: cpal::Stream,
    producer: rtrb::Producer<(f32, f32)>,
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
        })
    }

    pub fn push_samples(&mut self, samples: &[(f32, f32)]) {
        for &sample in samples {
            let _ = self.producer.push(sample);
        }
    }
}
