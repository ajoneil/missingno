use super::Channel;

pub struct WaveChannel {
    pub channel: Channel,
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            channel: Channel {
                enabled: false,
                output_left: true,
                output_right: false,
            },
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        self.channel = Channel::disabled();
    }
}
