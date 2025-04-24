pub struct LengthTimer {
    enabled: bool,
    timer: u8,
    initial_length: u8,
}

impl LengthTimer {
    const TRIGGER_AT: u8 = 64;
    const RATE_VS_AUDIO_TIMER: u8 = 2;

    pub fn new() -> Self {
        LengthTimer {
            enabled: false,
            timer: 0,
            initial_length: 0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_initial_length(&mut self, length: u8) {
        self.initial_length = length;
    }

    pub fn tick(&mut self) -> bool {
        if self.enabled {
            self.timer += 1;

            if self.expired() {
                self.disable();
                return true;
            }
        }

        false
    }

    pub fn enable(&mut self) {
        self.enabled = true;
        self.reset_timer()
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn trigger(&mut self) {
        if self.expired() {
            self.reset_timer();
        }
    }

    fn expired(&self) -> bool {
        self.timer == Self::TRIGGER_AT * Self::RATE_VS_AUDIO_TIMER
    }

    fn reset_timer(&mut self) {
        self.timer = self.initial_length * Self::RATE_VS_AUDIO_TIMER;
    }
}
