use super::Enabled;
use crate::emulator::audio::registers::VolumeAndEnvelope;

pub struct PulseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
}

impl Default for PulseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            volume_and_envelope: VolumeAndEnvelope(0xf3),
        }
    }
}

impl PulseChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
    }
}
