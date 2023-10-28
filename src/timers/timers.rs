use crate::{
    cpu::{Cycles, Interrupts},
    mmu::Mmu,
};

use super::cycle_timer::CycleTimer;

pub struct Timers {
    div: u8,
    div_timer: CycleTimer,

    counter: u8,
    modulo: u8,
    control: Control,
    timer: Option<CycleTimer>,
}

struct Control(u8);

impl Control {
    pub fn enabled(&self) -> bool {
        self.0 & 0b100 != 0
    }

    pub fn interval(&self) -> Cycles {
        match self.0 & 0b11 {
            0b00 => Cycles(1024),
            0b01 => Cycles(16),
            0b10 => Cycles(64),
            0b11.. => Cycles(256),
        }
    }
}

impl Timers {
    const DIV_INCREMENT_TIME: Cycles = Cycles(1024);

    pub fn new() -> Self {
        Self {
            div: 0xab,
            div_timer: CycleTimer::new(Self::DIV_INCREMENT_TIME),

            counter: 0,
            modulo: 0,
            control: Control(0xf8),
            timer: None,
        }
    }

    pub fn step(&mut self, cycles: Cycles, mmu: &mut Mmu) {
        self.div_timer.tick(cycles);
        while self.div_timer.finished() {
            self.div = if self.div == 0xff { 0 } else { self.div + 1 };
            self.div_timer.lap()
        }

        if let Some(timer) = &mut self.timer {
            timer.tick(cycles);
            while timer.finished() {
                if self.counter == 0xff {
                    self.counter = self.modulo;
                    mmu.set_interrupt_flag(Interrupts::TIMER)
                } else {
                    self.counter += 1
                }

                timer.lap()
            }
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xff04 => self.div,
            0xff05 => self.counter,
            0xff06 => self.modulo,
            0xff07 => self.control.0,

            _ => panic!("unimplemented timer read for address {:4x}", address),
        }
    }

    pub fn write(&mut self, address: u16, val: u8) {
        match address {
            0xff04 => {
                self.div = 0;
                self.div_timer.reset()
            }
            0xff05 => self.counter = val,
            0xff06 => self.modulo = val,
            0xff07 => {
                self.control = Control(val);
                self.timer = if self.control.enabled() {
                    Some(CycleTimer::new(self.control.interval()))
                } else {
                    None
                }
            }
            _ => panic!("unimplemented timer write for address {:4x}", address),
        }
    }
}
