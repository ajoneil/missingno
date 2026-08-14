//! A halted CPU wakes on its refetch grid: /INT is sampled at the rising
//! edge of each halt cycle's final T-state (Zilog UM), so the wake instant
//! quantises to 4 T from HALT retirement and the wake phase follows the
//! halt's entry phase.

use missingno_zilog_z80::{Bus, Cpu};

struct Ram(Vec<u8>);

impl Bus for Ram {
    fn read(&mut self, address: u16) -> u8 {
        self.0[address as usize]
    }
    fn write(&mut self, address: u16, data: u8) {
        self.0[address as usize] = data;
    }
    fn input(&mut self, _port: u16) -> u8 {
        0xFF
    }
    fn output(&mut self, _port: u16, _data: u8) {}
}

/// Ticks from HALT retirement to acceptance, with /INT raised `delay`
/// T-states after retirement.
fn wake_after(delay: u64) -> u64 {
    let mut ram = vec![0u8; 0x10000];
    // 0000: EI; IM 1; HALT.  0038: EI; RET.
    ram[0..4].copy_from_slice(&[0xFB, 0xED, 0x56, 0x76]);
    ram[0x38] = 0xFB;
    ram[0x39] = 0xC9;
    let mut bus = Ram(ram);
    let mut cpu = Cpu::new();

    let mut t: u64 = 0;
    loop {
        cpu.tick(&mut bus);
        t += 1;
        if cpu.halted && cpu.at_instruction_boundary() {
            break;
        }
        assert!(t < 100, "HALT never retired");
    }
    let halt_end = t;

    for _ in 0..delay {
        cpu.tick(&mut bus);
        t += 1;
    }
    cpu.set_irq(true);
    loop {
        cpu.tick(&mut bus);
        t += 1;
        if !cpu.halted {
            return t - halt_end;
        }
        assert!(t < halt_end + 100, "never woke");
    }
}

#[test]
fn halt_wake_rides_the_refetch_grid() {
    // A line up before the refetch cycle's final T rises is caught by that
    // cycle; one arriving at or after that edge waits out the next cycle.
    let wakes: Vec<u64> = (0..8).map(wake_after).collect();
    assert_eq!(wakes, [5, 5, 5, 5, 9, 9, 9, 9]);
}
