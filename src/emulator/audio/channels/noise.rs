use super::Channel;

pub struct NoiseChannel {
    pub channel: Channel,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            channel: Channel {
                enabled: false,
                output_left: true,
                output_right: true,
            },
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        self.channel = Channel::disabled()
    }
}
