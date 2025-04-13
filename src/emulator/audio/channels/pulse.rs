use super::Channel;

pub struct PulseChannel {
    pub channel: Channel,
}

impl Default for PulseChannel {
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

impl PulseChannel {
    pub fn reset(&mut self) {
        self.channel = Channel::disabled()
    }
}
