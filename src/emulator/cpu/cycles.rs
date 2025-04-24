#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
pub struct Cycles(pub u32);

impl std::ops::Add for Cycles {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::AddAssign for Cycles {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
    }
}

impl std::ops::SubAssign for Cycles {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs
    }
}
