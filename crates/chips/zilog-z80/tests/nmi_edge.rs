//! The /NMI pin's edge detector: a board driving the line as a level gets one
//! NMI per released-to-asserted transition, however long it holds the line.

use missingno_zilog_z80::{Bus, Cpu};

/// NOPs everywhere, so the NMI vector at $0066 runs straight on.
struct Nops;

impl Bus for Nops {
    fn read(&mut self, _address: u16) -> u8 {
        0x00
    }
    fn write(&mut self, _address: u16, _data: u8) {}
    fn input(&mut self, _port: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u16, _data: u8) {}
}

/// Step one instruction, reporting whether it was an NMI acceptance.
fn step_took_nmi(cpu: &mut Cpu) -> bool {
    let sp = cpu.sp;
    cpu.step(&mut Nops);
    cpu.pc == 0x0066 && cpu.sp == sp.wrapping_sub(2)
}

fn powered() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.sp = 0xDFF0;
    cpu.pc = 0x0100;
    cpu
}

#[test]
fn a_held_line_delivers_once() {
    let mut cpu = powered();
    cpu.set_nmi(true);
    let taken: Vec<bool> = (0..3).map(|_| step_took_nmi(&mut cpu)).collect();
    assert_eq!(taken, [true, false, false]);
}

#[test]
fn a_release_then_reassert_delivers_again() {
    let mut cpu = powered();
    cpu.set_nmi(true);
    assert!(step_took_nmi(&mut cpu));
    cpu.set_nmi(true);
    assert!(!step_took_nmi(&mut cpu));
    cpu.set_nmi(false);
    assert!(!step_took_nmi(&mut cpu));
    cpu.set_nmi(true);
    assert!(step_took_nmi(&mut cpu));
}

#[test]
fn a_triggered_pulse_still_delivers_once() {
    let mut cpu = powered();
    cpu.trigger_nmi();
    assert!(step_took_nmi(&mut cpu));
    assert!(!step_took_nmi(&mut cpu));
}
