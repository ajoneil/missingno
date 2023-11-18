use crate::emulation::cpu::Cycles;

pub fn nop() -> Cycles {
    Cycles(4)
}

pub fn di(ime: &mut bool) -> Cycles {
    println!("master interrupt disabled");
    *ime = false;
    Cycles(4)
}

pub fn ei(ime: &mut bool) -> Cycles {
    println!("master interrupt enabled");
    *ime = true;
    Cycles(4)
}
