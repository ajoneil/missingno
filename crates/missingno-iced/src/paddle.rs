//! Differential-rate paddle emulation for gamepad triggers: the right trigger
//! winds the paddle one way, the left the other, squeeze depth sets the speed,
//! and the knob holds its position when both are released — the couch
//! counterpart to driving the paddle from the pointer's absolute position.

/// Full-range sweeps per second at a fully squeezed trigger, roughly the wrist
/// speed of a hard twist on a real paddle.
const FULL_SQUEEZE_SWEEPS_PER_SEC: f32 = 2.0;

/// Trigger depressions below this read as released, so a slightly sticky
/// trigger doesn't creep the paddle.
const DEADZONE: f32 = 0.05;

#[derive(Debug, Clone)]
pub struct PaddleWind {
    position: f32,
    left: f32,
    right: f32,
}

impl PaddleWind {
    pub fn new() -> Self {
        Self {
            position: 0.5,
            left: 0.0,
            right: 0.0,
        }
    }

    pub fn set_left(&mut self, depression: f32) {
        self.left = zoned(depression);
    }

    pub fn set_right(&mut self, depression: f32) {
        self.right = zoned(depression);
    }

    /// The pointer path shares the knob: an absolute set keeps the two input
    /// styles continuous with each other.
    pub fn set_position(&mut self, position: f32) {
        self.position = position.clamp(0.0, 1.0);
    }

    pub fn idle(&self) -> bool {
        self.left == 0.0 && self.right == 0.0
    }

    /// Advance the knob by the elapsed time; `Some` carries the new position
    /// only while a trigger is actually winding it.
    pub fn tick(&mut self, dt: f32) -> Option<f32> {
        if self.idle() {
            return None;
        }
        let velocity = (self.right - self.left) * FULL_SQUEEZE_SWEEPS_PER_SEC;
        let wound = (self.position + velocity * dt).clamp(0.0, 1.0);
        if wound == self.position {
            return None;
        }
        self.position = wound;
        Some(wound)
    }
}

impl Default for PaddleWind {
    fn default() -> Self {
        Self::new()
    }
}

fn zoned(depression: f32) -> f32 {
    if depression < DEADZONE {
        0.0
    } else {
        depression.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_position_when_idle() {
        let mut wind = PaddleWind::new();
        assert_eq!(wind.tick(0.1), None);
        wind.set_right(1.0);
        wind.tick(0.1);
        wind.set_right(0.0);
        assert_eq!(wind.tick(0.1), None);
    }

    #[test]
    fn full_squeeze_sweeps_half_range_in_quarter_second() {
        let mut wind = PaddleWind::new();
        wind.set_right(1.0);
        assert_eq!(wind.tick(0.25), Some(1.0));
    }

    #[test]
    fn triggers_oppose() {
        let mut wind = PaddleWind::new();
        wind.set_left(0.6);
        wind.set_right(0.6);
        assert_eq!(wind.tick(0.1), None);
    }

    #[test]
    fn clamps_at_range_ends() {
        let mut wind = PaddleWind::new();
        wind.set_left(1.0);
        assert_eq!(wind.tick(10.0), Some(0.0));
        assert_eq!(wind.tick(0.1), None);
    }

    #[test]
    fn deadzone_ignores_sticky_triggers() {
        let mut wind = PaddleWind::new();
        wind.set_right(0.03);
        assert!(wind.idle());
    }

    #[test]
    fn pointer_set_is_absolute() {
        let mut wind = PaddleWind::new();
        wind.set_position(0.8);
        wind.set_right(1.0);
        assert!(wind.tick(0.05).unwrap() > 0.8);
    }
}
