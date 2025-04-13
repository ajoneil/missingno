pub struct PulseChannel {
    enabled: bool,
}

impl Default for PulseChannel {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl PulseChannel {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
