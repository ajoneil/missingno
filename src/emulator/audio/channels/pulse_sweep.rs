use super::Channel;

pub struct PulseSweepChannel {
    pub channel: Channel,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        Self {
            channel: Channel {
                enabled: true,
                output_left: true,
                output_right: true,
            },
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        self.channel = Channel::disabled();
    }
}
