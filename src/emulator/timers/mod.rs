use crate::emulator::{audio::Audio, cpu::cycles::Cycles, interrupts::Interrupt};
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

    system_timer: u16,
    timer: Option<CycleTimer>,
}

const AUDIO_DIVIDER_WATCH_BIT: u8 = 0b0001_0000;

impl Timers {
    pub fn new() -> Self {
        Self {
            divider: 0xab,
            counter: 0,
            modulo: 0,
            control: Control(0xf8),

            system_timer: 0,
            timer: None,
        }
    }

    pub fn step(&mut self, cycles: Cycles, audio: &mut Audio) -> Option<Interrupt> {
        for c in 0..cycles.0 {
            if self.update_system_timer_check_audio(self.system_timer.wrapping_add(1)) {
                audio.trigger_audio_timer(Cycles(c));
            }
        }

        if let Some(timer) = &mut self.timer {
            timer.tick(cycles);
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

    pub fn write_register(&mut self, register: Register, value: u8, audio: &mut Audio) {
        match register {
            Register::Divider => {
                if self.update_system_timer_check_audio(0) {
                    dbg!("tick");
                    audio.trigger_audio_timer(Cycles(0));
                }
            }
            Register::Counter => self.counter = value,
            Register::Modulo => self.modulo = value,
            Register::Control => {
                self.control = Control(value);
                if self.control.enabled() {
                    self.timer = Some(CycleTimer::new(self.control.interval()));
                } else {
                    self.timer = None;
                }
            }
        }
    }

    fn update_system_timer_check_audio(&mut self, value: u16) -> bool {
        let before = self.divider();
        self.system_timer = value;
        before & AUDIO_DIVIDER_WATCH_BIT != 0 && self.divider() & AUDIO_DIVIDER_WATCH_BIT == 0
    }

    fn divider(&self) -> u8 {
        ((self.system_timer >> 6) & 0xff) as u8
    }
}
