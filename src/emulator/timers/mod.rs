use crate::emulator::interrupts::Interrupt;
use cycle_timer::CycleTimer;
use registers::Control;
pub use registers::Register;

pub mod cycle_timer;
pub mod registers;

pub struct Timers {
    divider: u8,
    counter: u8,
    modulo: u8,
    control: Control,

    divider_timer: CycleTimer,
    timer: Option<CycleTimer>,
}

impl Timers {
    const DIV_INCREMENT_CYCLES: u32 = 1024;

    pub fn new() -> Self {
        Self {
            divider: 0xab,
            counter: 0,
            modulo: 0,
            control: Control(0xf8),

            divider_timer: CycleTimer::new(Self::DIV_INCREMENT_CYCLES),
            timer: None,
        }
    }

    pub fn tick(&mut self) -> Option<Interrupt> {
        self.divider_timer.tick();
        if self.divider_timer.finished() {
            self.divider = self.divider.wrapping_add(1);
            self.divider_timer.lap()
        }

        if let Some(timer) = &mut self.timer {
            timer.tick();
            if timer.finished() {
                timer.lap();

                if self.counter == 0xff {
                    self.counter = self.modulo;
                    return Some(Interrupt::Timer);
                } else {
                    self.counter += 1;
                }
            }
        }

        None
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Divider => self.divider,
            Register::Counter => self.counter,
            Register::Modulo => self.modulo,
            Register::Control => self.control.0,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Divider => self.divider = 0,
            Register::Counter => self.counter = value,
            Register::Modulo => self.modulo = value,
            Register::Control => {
                self.control = Control(value);
                if self.control.enabled() {
                    self.timer = Some(CycleTimer::new(self.control.cycle_interval()));
                } else {
                    self.timer = None;
                }
            }
        }
    }
}
