#[derive(Copy, Clone)]
pub struct Volume(pub u8);

impl Volume {
    pub fn max() -> Self {
        Self(7)
    }

    // Volumes go from 0 (quietest) to 7 (loudest). 0 is not muted.
    // pub fn percentage(self) -> f32 {
    //     (self.0 + 1) as f32 / 8.0
    // }
}
