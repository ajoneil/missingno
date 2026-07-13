//! MOS 6532 RIOT: 128 bytes of RAM, the interval timer, and two I/O ports.
//!
//! Timer semantics are datasheet-grounded (no die sim exists for the RIOT —
//! the one silicon-blind chip): the counter decrements once per interval;
//! after underflow it free-runs at one step per CPU cycle until INTIM is
//! read, which also clears the underflow flag and re-arms the interval.

pub struct Riot {
    pub ram: [u8; 128],
    timer: u8,
    /// Prescaler divisor selected by TIM1T/TIM8T/TIM64T/TIM1024T (1/8/64/1024).
    interval: u16,
    prescaler: u16,
    timer_underflowed: bool,
    /// Underflow landed at the last tick; an INTIM read this cycle coincides
    /// with the interrupt and must not clear the flag (datasheet exception).
    timer_just_underflowed: bool,
    /// Output registers (ORA/ORB) hold what software wrote, whole, across DDR flips.
    output_a: u8,
    output_b: u8,
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
            timer_just_underflowed: false,
            output_a: 0,
            output_b: 0,
            pins_a: 0xFF,
            // Reset/Select released, Color mode, both difficulties Beginner.
            pins_b: 0x0B,
            ddr_a: 0,
            ddr_b: 0,
            pa7_flag: false,
            pa7_positive_edge: false,
        }
    }

    /// A port-A pin driven from outside (joysticks).
    pub fn set_pin_a(&mut self, mask: u8, high: bool) {
        let before = self.pa7_level();
        if high {
            self.pins_a |= mask;
        } else {
            self.pins_a &= !mask;
        }
        self.pa7_edge(before);
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
        (self.output_a & self.ddr_a) | (self.pins_a & !self.ddr_a)
    }

    fn port_b_pins(&self) -> u8 {
        (self.output_b & self.ddr_b) | (self.pins_b & !self.ddr_b)
    }

    /// The edge detect watches the PA7 pin — which follows ORA when the
    /// line is an output, so software can raise the flag by itself.
    fn pa7_level(&self) -> bool {
        self.port_a_pins() & 0x80 != 0
    }

    fn pa7_edge(&mut self, was_high: bool) {
        let is_high = self.pa7_level();
        if was_high != is_high && is_high == self.pa7_positive_edge {
            self.pa7_flag = true;
        }
    }

    fn interrupt_flags(&self) -> u8 {
        let timer = if self.timer_underflowed { 0x80 } else { 0x00 };
        let pa7 = if self.pa7_flag { 0x40 } else { 0x00 };
        timer | pa7
    }

    /// One CPU-clock tick.
    pub fn tick(&mut self) {
        self.timer_just_underflowed = false;
        if self.timer_underflowed {
            self.timer = self.timer.wrapping_sub(1);
            return;
        }
        self.prescaler += 1;
        if self.prescaler >= self.interval {
            self.prescaler = 0;
            if self.timer == 0 {
                self.timer_underflowed = true;
                self.timer_just_underflowed = true;
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
                if self.timer_underflowed && !self.timer_just_underflowed {
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
            let selected_positive = register & 0x01 != 0;
            // A polarity change acts as if the new active edge just landed:
            // with PA7 already at its post-edge level, the flag sets
            // (datasheet warning; the exact matrix is hardware-unconfirmed).
            if selected_positive != self.pa7_positive_edge && self.pa7_level() == selected_positive
            {
                self.pa7_flag = true;
            }
            self.pa7_positive_edge = selected_positive;
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
            0x00 | 0x01 => {
                let before = self.pa7_level();
                if register & 0x07 == 0x00 {
                    self.output_a = value;
                } else {
                    self.ddr_a = value;
                }
                self.pa7_edge(before);
            }
            0x02 => self.output_b = value,
            0x03 => self.ddr_b = value,
            _ => {}
        }
    }
}
