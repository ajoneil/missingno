pub struct NoiseChannel {
    enabled: bool,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl NoiseChannel {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
