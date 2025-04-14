use super::Enabled;
use crate::emulator::audio::registers::VolumeAndEnvelope;

pub struct PulseSweepChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: true,
                output_left: true,
                output_right: true,
            },
            volume_and_envelope: VolumeAndEnvelope(0),
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
    }
}
