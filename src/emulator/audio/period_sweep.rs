pub struct PeriodSweep {
    register: u8,
    enabled: bool,
    timer: u8,
}

pub enum Direction {
    Increasing,
    Decreasing,
}

impl PeriodSweep {
    pub fn new(register: u8) -> Self {
        Self {
            register,
            enabled: false,
            timer: 0,
        }
    }

    pub fn pace(&self) -> u8 {
        (self.register & 0b0111_0000) >> 4
    }

    pub fn direction(&self) -> Direction {
        if self.register & 0b1000 != 0 {
            Direction::Increasing
        } else {
            Direction::Decreasing
        }
    }

    pub fn individual_step(&self) -> u8 {
        self.register & 0b111
    }

    pub fn read_register(&self) -> u8 {
        self.register
    }

    pub fn write_register(&mut self, value: u8) {
        self.register = value;
    }

    pub fn calculate_period(&mut self, current_period: u16) -> u16 {
        let change = match self.direction() {
            Direction::Increasing => current_period >> self.individual_step(),
            Direction::Decreasing => !(current_period >> self.individual_step()) & 0x7ff,
        };

        let new_period = current_period + change;
        if new_period > 0x7ff {
            self.enabled = false;
        }

        new_period
    }

    pub fn trigger(&mut self, current_period: u16) {
        self.enabled = self.pace() > 0 || self.individual_step() > 0;
        self.timer = 0;
        if self.individual_step() > 0 {
            // Calculate and check overflow only - don't modify frequency
            self.calculate_period(current_period);
        }
    }

    pub fn tick(&mut self, current_period: u16) -> Option<u16> {
        self.timer += 1;

        if self.timer == 4 {
            self.timer = 0;

            if self.enabled && self.pace() > 0 {
                let new_period = self.calculate_period(current_period);
                if new_period <= 0x7ff {
                    // Check overflow again but don't modify frequency
                    self.calculate_period(new_period);

                    return Some(new_period);
                }
            }
        }

        None
    }
}
