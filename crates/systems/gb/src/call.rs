//! Calling into a routine from the host, as a debugger's "call" does: the
//! routine runs on the real CPU as though a CALL at the current instruction
//! boundary had jumped to it, and the call ends when it returns there.

use crate::execute::StepResult;
use crate::snapshot::capture_cpu;
use crate::{Console, Model, cpu::Cpu};

/// The SM83's 8-bit registers, which a routine takes its arguments in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    /// Only the upper nibble (Z, N, H, C) exists; the rest reads zero.
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// The routine returned to the boundary the call was made from.
    Returned { tcycles: u32 },
    /// The budget ran out first; the CPU is left wherever the routine was.
    OverBudget { tcycles: u32 },
}

impl CallOutcome {
    pub fn tcycles(self) -> u32 {
        match self {
            CallOutcome::Returned { tcycles } | CallOutcome::OverBudget { tcycles } => tcycles,
        }
    }
}

impl<M: Model> Console<M> {
    /// Call `routine` with `registers` loaded, pushing the address of the
    /// instruction the CPU was about to run onto the stack at SP. Runs whole
    /// instructions until the routine returns there with SP back where it
    /// was, or until `budget` T-cycles have passed. Interrupt enables are left
    /// as they are. `observe` sees the console after each instruction, with
    /// that instruction's accesses in [`Console::bus_trace`].
    pub fn call(
        &mut self,
        routine: u16,
        registers: Registers,
        budget: u32,
        mut observe: impl FnMut(&Self, &StepResult),
    ) -> CallOutcome {
        assert!(
            self.chassis.cpu.at_instruction_boundary(),
            "a call is made between instructions"
        );
        let return_address = self.chassis.cpu.ir_address;
        let caller_sp = self.chassis.cpu.stack_pointer;
        let [low, high] = return_address.to_le_bytes();
        let sp = caller_sp.wrapping_sub(2);
        self.write_byte_with_write_strobe_lock(sp.wrapping_add(1), high, None, None);
        self.write_byte_with_write_strobe_lock(sp, low, None, None);

        let mut entry = capture_cpu(self);
        let Registers {
            a,
            f,
            b,
            c,
            d,
            e,
            h,
            l,
        } = registers;
        (entry.a, entry.f, entry.b, entry.c) = (a, f & 0xf0, b, c);
        (entry.d, entry.e, entry.h, entry.l) = (d, e, h, l);
        (entry.sp, entry.pc) = (sp, routine);
        entry.halt_state = 0;
        entry.halt_bug = false;
        entry.halt_latched = false;
        entry.irq_latched = false;
        entry.dispatching = false;
        self.install_cpu(Cpu::from_snapshot(&entry));

        let mut tcycles = 0;
        loop {
            let result = self.step_recorded();
            tcycles += result.tcycles;
            observe(self, &result);
            let cpu = &self.chassis.cpu;
            if cpu.ir_address == return_address && cpu.stack_pointer == caller_sp {
                return CallOutcome::Returned { tcycles };
            }
            if tcycles >= budget {
                return CallOutcome::OverBudget { tcycles };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameBoy;
    use crate::cartridge::Cartridge;

    const SUBROUTINE: u16 = 0x0160;

    /// A program looping on `CALL $0160; JR -5`, where $0160 is `INC D; RET`,
    /// stopped as it enters $0160 so a call returns into the middle of it.
    fn console_with(routine_at: u16, routine: &[u8]) -> GameBoy {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x104].copy_from_slice(&[0x00, 0xc3, 0x50, 0x01]);
        rom[0x150..0x155].copy_from_slice(&[0xcd, 0x60, 0x01, 0x18, 0xfb]);
        rom[0x160..0x162].copy_from_slice(&[0x14, 0xc9]);
        let at = routine_at as usize;
        rom[at..at + routine.len()].copy_from_slice(routine);
        let mut console = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
        while console.cpu().ir_address != SUBROUTINE {
            console.step();
        }
        console
    }

    #[test]
    fn a_call_returns_through_a_reentry_of_its_return_address() {
        // CALL $0160; INC E; LD A,C; ADD A,B; RET
        let mut console = console_with(0x0200, &[0xcd, 0x60, 0x01, 0x1c, 0x79, 0x80, 0xc9]);
        let sp = console.cpu().stack_pointer;
        let registers = Registers {
            b: 0x12,
            c: 0x30,
            ..Registers::default()
        };
        let mut instructions = 0;
        let outcome = console.call(0x0200, registers, 1000, |_, _| instructions += 1);

        // CALL 24, INC D 4, RET 16, INC E 4, LD 4, ADD 4, RET 16
        assert_eq!(outcome, CallOutcome::Returned { tcycles: 72 });
        assert_eq!(instructions, 7);
        let cpu = console.cpu();
        assert_eq!((cpu.a, cpu.d, cpu.e), (0x42, 1, 1));
        assert_eq!((cpu.ir_address, cpu.stack_pointer), (SUBROUTINE, sp));
    }

    #[test]
    fn a_call_that_never_returns_stops_at_its_budget() {
        let mut console = console_with(0x0200, &[0x18, 0xfe]);
        let outcome = console.call(0x0200, Registers::default(), 1000, |_, _| {});
        assert!(
            matches!(outcome, CallOutcome::OverBudget { tcycles } if (1000..1012).contains(&tcycles))
        );
        assert_eq!(console.cpu().ir_address, 0x0200);
    }

    #[test]
    fn a_call_is_observed_on_the_bus() {
        // LD ($C000),A; RET
        let mut console = console_with(0x0200, &[0xea, 0x00, 0xc0, 0xc9]);
        let mut writes = Vec::new();
        let registers = Registers {
            a: 0x5a,
            ..Registers::default()
        };
        console.call(0x0200, registers, 1000, |console, _| {
            writes.extend(
                console
                    .bus_trace()
                    .iter()
                    .filter(|access| matches!(access.kind, crate::cpu_bus::BusAccessKind::Write))
                    .map(|access| (access.address, access.value)),
            );
        });
        assert_eq!(writes, [(0xc000, 0x5a)]);
        assert_eq!(console.peek(0xc000), 0x5a);
    }

    #[test]
    fn a_call_can_be_made_before_the_first_instruction() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x200] = 0xc9;
        let mut console = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
        let outcome = console.call(0x0200, Registers::default(), 1000, |_, _| {});
        assert_eq!(outcome, CallOutcome::Returned { tcycles: 16 });
        assert_eq!(console.cpu().ir_address, 0x0100);
    }
}
