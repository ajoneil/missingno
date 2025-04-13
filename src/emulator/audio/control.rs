use bitflags::bitflags;

bitflags! {
    pub struct ControlFlags: u8 {
        const AUDIO_ENABLE  = 0b1000_0000;
        const CHANNEL_4_ON  = 0b0000_1000;
        const CHANNEL_3_ON  = 0b0000_0100;
        const CHANNEL_2_ON  = 0b0000_0010;
        const CHANNEL_1_ON  = 0b0000_0001;
    }
}
