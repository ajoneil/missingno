use crate::cpu::Cycles;

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

    pub fn counted(&self) -> Cycles {
        self.counted
    }

    pub fn finished(&self) -> bool {
        self.counted >= self.length
    }

    pub fn reset(&mut self) {
        self.counted = Cycles(0)
    }

    pub fn lap(&mut self) {
        assert!(self.finished());
        self.counted -= self.length
    }

    pub fn overflow(&self) -> Option<Cycles> {
        if self.counted > self.length {
            Some(self.counted - self.length)
        } else {
            None
        }
    }
}
