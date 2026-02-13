use bitflags::bitflags;

use crate::game_boy::audio::{
    Audio, Register,
    channels::{noise, pulse, pulse_sweep, wave},
    volume::Volume,
};

impl Register {
    pub fn map(address: u16) -> Self {
        match address {
            0xff10 => Self::Channel1(pulse_sweep::Register::PeriodSweep),
            0xff11 => Self::Channel1(pulse_sweep::Register::WaveformAndInitialLength),
            0xff12 => Self::Channel1(pulse_sweep::Register::Volume),
            0xff13 => Self::Channel1(pulse_sweep::Register::PeriodLow),
            0xff14 => Self::Channel1(pulse_sweep::Register::PeriodHighAndControl),

            0xff16 => Self::Channel2(pulse::Register::WaveformAndInitialLength),
            0xff17 => Self::Channel2(pulse::Register::VolumeAndEnvelope),
            0xff18 => Self::Channel2(pulse::Register::PeriodLow),
            0xff19 => Self::Channel2(pulse::Register::PeriodHighAndControl),

            0xff1a => Self::Channel3(wave::Register::DacEnabled),
            0xff1b => Self::Channel3(wave::Register::Length),
            0xff1c => Self::Channel3(wave::Register::Volume),
            0xff1d => Self::Channel3(wave::Register::PeriodLow),
            0xff1e => Self::Channel3(wave::Register::PeriodHighAndControl),

            0xff20 => Self::Channel4(noise::Register::LengthTimer),
            0xff21 => Self::Channel4(noise::Register::VolumeAndEnvelope),
            0xff22 => Self::Channel4(noise::Register::FrequencyAndRandomness),
            0xff23 => Self::Channel4(noise::Register::Control),

            0xff24 => Self::Volume,
            0xff25 => Self::Panning,
            0xff26 => Self::Control,
            _ => todo!("unmapped audio register {:04x}", address),
        }
    }
}

impl Audio {
    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => {
                if self.enabled {
                    let mut value = ControlFlags::AUDIO_ENABLE;
                    value.set(
                        ControlFlags::CHANNEL_1_ON,
                        self.channels.ch1.enabled.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_2_ON,
                        self.channels.ch2.enabled.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_3_ON,
                        self.channels.ch3.enabled.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_4_ON,
                        self.channels.ch4.enabled.enabled,
                    );

                    value.bits() | 0x70
                } else {
                    0x70
                }
            }

            Register::Panning => {
                let mut value = PanFlags::empty();
                value.set(
                    PanFlags::CHANNEL_1_LEFT,
                    self.channels().ch1.enabled.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_1_RIGHT,
                    self.channels().ch1.enabled.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_2_LEFT,
                    self.channels().ch2.enabled.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_2_RIGHT,
                    self.channels().ch2.enabled.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_3_LEFT,
                    self.channels().ch3.enabled.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_3_RIGHT,
                    self.channels().ch3.enabled.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_4_LEFT,
                    self.channels().ch4.enabled.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_4_RIGHT,
                    self.channels().ch4.enabled.output_right,
                );

                value.bits()
            }

            Register::Volume => self.nr50,
            Register::Channel1(register) => self.channels.ch1.read_register(register),
            Register::Channel2(register) => self.channels.ch2.read_register(register),
            Register::Channel3(register) => self.channels.ch3.read_register(register),
            Register::Channel4(register) => self.channels.ch4.read_register(register),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        if !self.enabled {
            match register {
                Register::Control => {} // always writable
                // DMG: NRx1 length timer registers are writable even when APU is off.
                // For NR11/NR21, only the length bits (lower 6) take effect;
                // the duty bits (upper 2) are masked out.
                Register::Channel1(pulse_sweep::Register::WaveformAndInitialLength)
                | Register::Channel2(pulse::Register::WaveformAndInitialLength) => {
                    let length_only = value & 0x3F;
                    match register {
                        Register::Channel1(r) => {
                            self.channels.ch1.write_register(
                                r,
                                length_only,
                                self.frame_sequencer_step,
                            );
                        }
                        Register::Channel2(r) => {
                            self.channels.ch2.write_register(
                                r,
                                length_only,
                                self.frame_sequencer_step,
                            );
                        }
                        _ => unreachable!(),
                    }
                    return;
                }
                Register::Channel3(wave::Register::Length)
                | Register::Channel4(noise::Register::LengthTimer) => {}
                _ => return,
            }
        }

        match register {
            Register::Control => {
                if ControlFlags::from_bits_retain(value).contains(ControlFlags::AUDIO_ENABLE) {
                    if !self.enabled {
                        // APU power-on: reset frame sequencer step only.
                        // The DIV-APU bit tracking is NOT reset — the frame
                        // sequencer picks up wherever DIV currently is.
                        self.frame_sequencer_step = 0;
                    }
                    self.enabled = true;
                } else {
                    self.enabled = false;
                    self.channels.ch1.reset();
                    self.channels.ch2.reset();
                    self.channels.ch3.reset();
                    self.channels.ch4.reset();
                    self.nr50 = 0;
                    self.volume_left = Volume(0);
                    self.volume_right = Volume(0);
                }
            }
            Register::Panning => {
                let value = PanFlags::from_bits_truncate(value);
                self.channels.ch1.enabled.output_left = value.contains(PanFlags::CHANNEL_1_LEFT);
                self.channels.ch1.enabled.output_right = value.contains(PanFlags::CHANNEL_1_RIGHT);
                self.channels.ch2.enabled.output_left = value.contains(PanFlags::CHANNEL_2_LEFT);
                self.channels.ch2.enabled.output_right = value.contains(PanFlags::CHANNEL_2_RIGHT);
                self.channels.ch3.enabled.output_left = value.contains(PanFlags::CHANNEL_3_LEFT);
                self.channels.ch3.enabled.output_right = value.contains(PanFlags::CHANNEL_3_RIGHT);
                self.channels.ch4.enabled.output_left = value.contains(PanFlags::CHANNEL_4_LEFT);
                self.channels.ch4.enabled.output_right = value.contains(PanFlags::CHANNEL_4_RIGHT);
            }
            Register::Volume => {
                self.nr50 = value;
                self.volume_left = Volume((value >> 4) & 0b111);
                self.volume_right = Volume(value & 0b111);
            }
            Register::Channel1(register) => {
                self.channels
                    .ch1
                    .write_register(register, value, self.frame_sequencer_step)
            }
            Register::Channel2(register) => {
                self.channels
                    .ch2
                    .write_register(register, value, self.frame_sequencer_step)
            }
            Register::Channel3(register) => {
                self.channels
                    .ch3
                    .write_register(register, value, self.frame_sequencer_step)
            }
            Register::Channel4(register) => {
                self.channels
                    .ch4
                    .write_register(register, value, self.frame_sequencer_step)
            }
        }
    }
}

bitflags! {
    pub struct ControlFlags: u8 {
        const AUDIO_ENABLE  = 0b1000_0000;
        const CHANNEL_4_ON  = 0b0000_1000;
        const CHANNEL_3_ON  = 0b0000_0100;
        const CHANNEL_2_ON  = 0b0000_0010;
        const CHANNEL_1_ON  = 0b0000_0001;
    }
}

bitflags! {
    pub struct PanFlags : u8 {
        const CHANNEL_4_LEFT  = 0b1000_0000;
        const CHANNEL_3_LEFT  = 0b0100_0000;
        const CHANNEL_2_LEFT  = 0b0010_0000;
        const CHANNEL_1_LEFT  = 0b0001_0000;
        const CHANNEL_4_RIGHT = 0b0000_1000;
        const CHANNEL_3_RIGHT = 0b0000_0100;
        const CHANNEL_2_RIGHT = 0b0000_0010;
        const CHANNEL_1_RIGHT = 0b0000_0001;
    }
}
