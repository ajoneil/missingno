use nanoserde::{DeRon, SerRon};

#[derive(Clone, SerRon, DeRon)]
pub struct CycleTimer {
    cycles: u32,
    counted: u32,
}

impl CycleTimer {
    pub fn new(cycles: u32) -> Self {
        Self { cycles, counted: 0 }
    }

    pub fn tick(&mut self) {
        self.counted += 1;
    }

    pub fn finished(&self) -> bool {
        self.counted >= self.cycles
    }

    pub fn lap(&mut self) {
        assert!(self.finished());
        self.counted -= self.cycles
    }
}
