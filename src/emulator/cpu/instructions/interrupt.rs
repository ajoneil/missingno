use core::fmt;

pub enum Interrupt {
    Enable,
    Disable,
    Await,
}

impl fmt::Display for Interrupt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Enable => write!(f, "ei"),
            Self::Disable => write!(f, "di"),
            Self::Await => write!(f, "halt"),
        }
    }
}
