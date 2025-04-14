use super::Enabled;
use crate::emulator::audio::registers::VolumeAndEnvelope;

pub struct NoiseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            volume_and_envelope: VolumeAndEnvelope(0),
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
    }
}
