pub struct PulseSweepChannel {
    enabled: bool,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl PulseSweepChannel {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
