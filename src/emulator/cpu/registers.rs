use core::fmt;

#[derive(Clone, Copy)]
pub enum Register8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl fmt::Display for Register8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::A => "a",
                Self::B => "b",
                Self::C => "c",
                Self::D => "d",
                Self::E => "e",
                Self::H => "h",
                Self::L => "l",
            }
        )
    }
}

#[derive(Clone, Copy)]
pub enum Register16 {
    Bc,
    De,
    Hl,
    StackPointer,
    Af,
}

impl fmt::Display for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Bc => "bc",
                Self::De => "de",
                Self::Hl => "hl",
                Self::StackPointer => "sp",
                Self::Af => "af",
            }
        )
    }
}
