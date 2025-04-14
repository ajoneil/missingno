use super::Enabled;

pub struct Volume(pub u8);
impl Volume {
    pub fn volume(&self) -> f32 {
        ((self.0 >> 5) & 0b11) as f32 / 4.0
    }
}

pub struct WaveChannel {
    pub enabled: Enabled,
    pub volume: Volume,
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: false,
            },
            volume: Volume(0x9f),
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume = Volume(0);
    }
}
