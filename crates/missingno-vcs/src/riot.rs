//! MOS 6532 RIOT: 128 bytes of RAM, the interval timer, and two I/O ports.
//!
//! Timer semantics are datasheet-grounded (no die sim exists for the RIOT —
//! the one silicon-blind chip): the counter decrements once per interval;
//! after underflow it free-runs at one step per CPU cycle until INTIM is
//! read, which also clears the underflow flag and re-arms the interval.

pub struct Riot {
    pub ram: [u8; 128],
    timer: u8,
    interval: u16,
    prescaler: u16,
    timer_underflowed: bool,
    /// Joystick lines, active-low: port 1 in the high nibble.
    pub port_a: u8,
    /// Console switches, active-low momentaries (see `switches`).
    pub port_b: u8,
    ddr_a: u8,
    ddr_b: u8,
}

impl Default for Riot {
    fn default() -> Self {
        Self::new()
    }
}

impl Riot {
    pub fn new() -> Self {
        Riot {
            ram: [0; 128],
            timer: 0,
            interval: 1024,
            prescaler: 0,
            timer_underflowed: false,
            port_a: 0xFF,
            // Reset/Select released, Color mode, both difficulties Beginner.
            port_b: 0x0B,
            ddr_a: 0,
            ddr_b: 0,
        }
    }

    /// One CPU-clock tick.
    pub fn tick(&mut self) {
        if self.timer_underflowed {
            self.timer = self.timer.wrapping_sub(1);
            return;
        }
        self.prescaler += 1;
        if self.prescaler >= self.interval {
            self.prescaler = 0;
            if self.timer == 0 {
                self.timer_underflowed = true;
                self.timer = 0xFF;
            } else {
                self.timer -= 1;
            }
        }
    }

    /// Inspection read: no underflow-flag clearing, no re-arming.
    pub fn peek(&self, register: u16) -> u8 {
        match register & 0x07 {
            0x00 => self.port_a,
            0x01 => self.ddr_a,
            0x02 => self.port_b,
            0x03 => self.ddr_b,
            0x05 | 0x07 => {
                if self.timer_underflowed {
                    0x80
                } else {
                    0x00
                }
            }
            _ => self.timer,
        }
    }

    pub fn read(&mut self, register: u16) -> u8 {
        match register & 0x07 {
            0x00 => self.port_a,
            0x01 => self.ddr_a,
            0x02 => self.port_b,
            0x03 => self.ddr_b,
            // Reading the interrupt-flag register leaves the timer flag
            // intact; only timer-register accesses clear it.
            0x05 | 0x07 => {
                if self.timer_underflowed {
                    0x80
                } else {
                    0x00
                }
            }
            _ => {
                let value = self.timer;
                if self.timer_underflowed {
                    self.timer_underflowed = false;
                    self.prescaler = 0;
                }
                value
            }
        }
    }

    pub fn write(&mut self, register: u16, value: u8) {
        if register & 0x14 == 0x14 {
            self.interval = match register & 0x03 {
                0x00 => 1,
                0x01 => 8,
                0x02 => 64,
                _ => 1024,
            };
            self.timer = value;
            // First decrement one clock after the write: underflow lands
            // at (value x divisor) + 1 clocks.
            self.prescaler = self.interval - 1;
            self.timer_underflowed = false;
            return;
        }
        match register & 0x07 {
            0x00 => self.port_a = value | !self.ddr_a,
            0x01 => self.ddr_a = value,
            0x02 => self.port_b = value | !self.ddr_b,
            0x03 => self.ddr_b = value,
            _ => {}
        }
    }
}
