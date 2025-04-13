pub struct WaveChannel {
    enabled: bool,
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl WaveChannel {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
