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
    /// Output registers hold what software wrote, whole, across DDR flips.
    ora: u8,
    orb: u8,
    /// External pin levels: joystick lines on A (active-low, port 1 in the
    /// high nibble), console switches on B (active-low momentaries).
    pins_a: u8,
    pins_b: u8,
    ddr_a: u8,
    ddr_b: u8,
    /// PA7 edge detect: flag set on the configured pin transition.
    pa7_flag: bool,
    pa7_positive_edge: bool,
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
            ora: 0,
            orb: 0,
            pins_a: 0xFF,
            // Reset/Select released, Color mode, both difficulties Beginner.
            pins_b: 0x0B,
            ddr_a: 0,
            ddr_b: 0,
            pa7_flag: false,
            pa7_positive_edge: false,
        }
    }

    /// A port-A pin driven from outside (joysticks); PA7 transitions feed
    /// the edge detect.
    pub fn set_pin_a(&mut self, mask: u8, high: bool) {
        let before = self.pins_a;
        if high {
            self.pins_a |= mask;
        } else {
            self.pins_a &= !mask;
        }
        let was_high = before & 0x80 != 0;
        let is_high = self.pins_a & 0x80 != 0;
        if was_high != is_high && is_high == self.pa7_positive_edge {
            self.pa7_flag = true;
        }
    }

    /// A port-B pin driven from outside (console switches).
    pub fn set_pin_b(&mut self, mask: u8, high: bool) {
        if high {
            self.pins_b |= mask;
        } else {
            self.pins_b &= !mask;
        }
    }

    /// Reads return pin levels: output bits from the register, input bits
    /// from the outside world.
    fn port_a_pins(&self) -> u8 {
        (self.ora & self.ddr_a) | (self.pins_a & !self.ddr_a)
    }

    fn port_b_pins(&self) -> u8 {
        (self.orb & self.ddr_b) | (self.pins_b & !self.ddr_b)
    }

    fn interrupt_flags(&self) -> u8 {
        let timer = if self.timer_underflowed { 0x80 } else { 0x00 };
        let pa7 = if self.pa7_flag { 0x40 } else { 0x00 };
        timer | pa7
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

    /// Inspection read: no flag clearing, no re-arming.
    pub fn peek(&self, register: u16) -> u8 {
        match register & 0x07 {
            0x00 => self.port_a_pins(),
            0x01 => self.ddr_a,
            0x02 => self.port_b_pins(),
            0x03 => self.ddr_b,
            0x05 | 0x07 => self.interrupt_flags(),
            _ => self.timer,
        }
    }

    pub fn read(&mut self, register: u16) -> u8 {
        match register & 0x07 {
            0x00 => self.port_a_pins(),
            0x01 => self.ddr_a,
            0x02 => self.port_b_pins(),
            0x03 => self.ddr_b,
            // Reading the flag register clears the PA7 flag and leaves the
            // timer flag intact; only timer-register accesses clear that.
            0x05 | 0x07 => {
                let flags = self.interrupt_flags();
                self.pa7_flag = false;
                flags
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
        // A4=0, A2=1: PA7 edge-detect control — A0 picks the active edge
        // (the IRQ-enable bit gates a pin the 6507 package doesn't have).
        if register & 0x14 == 0x04 {
            self.pa7_positive_edge = register & 0x01 != 0;
            return;
        }
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
            0x00 => self.ora = value,
            0x01 => self.ddr_a = value,
            0x02 => self.orb = value,
            0x03 => self.ddr_b = value,
            _ => {}
        }
    }
}
