use core::fmt;

pub enum CarryFlag {
    Complement,
    Set,
}

impl fmt::Display for CarryFlag {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Complement => write!(f, "ccf"),
            Self::Set => write!(f, "scf"),
        }
    }
}
