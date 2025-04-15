use crate::emulator::cpu::cycles::Cycles;

pub struct CycleTimer {
    length: Cycles,
    counted: Cycles,
}

impl CycleTimer {
    pub fn new(length: Cycles) -> Self {
        Self {
            length,
            counted: Cycles(0),
        }
    }

    pub fn tick(&mut self, delta: Cycles) {
        self.counted += delta;
    }

    pub fn finished(&self) -> bool {
        self.counted >= self.length
    }

    pub fn lap(&mut self) {
        assert!(self.finished());
        self.counted -= self.length
    }
}
