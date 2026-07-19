//! Debugging backend: instruction stepping, PC breakpoints, and bus-access
//! watchpoints over a console, with side-effect-free inspection through
//! [`Vcs::peek`].

use std::collections::BTreeSet;

use missingno_6502::Mos6502;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;

use crate::console::{Frame, Vcs};

/// A JSR opcode, for step-over.
const JSR: u8 = 0x20;

// Synthetic address bases above the real bus, where the debugger exposes
// bank-complete cartridge stores past the board's paged window. The scheme
// mirrors the Game Boy debugger's: each store gets its own decade with room for
// the largest image the family allows.
/// Bank-complete cart RAM, all banks linear.
const CART_RAM_BASE: u32 = 0x0100_0000;
/// The full ROM image, all banks in file order.
const CART_ROM_BASE: u32 = 0x0200_0000;

/// Named bits of the 6502 status register `p`; the B flag is not architectural.
pub(crate) const MOS6502_FLAGS: &[inspect::FlagName] = &[
    inspect::FlagName {
        name: "n",
        bit: 7,
        help: Some("negative flag — bit 7 of the result"),
    },
    inspect::FlagName {
        name: "v",
        bit: 6,
        help: Some("overflow flag — signed overflow"),
    },
    inspect::FlagName {
        name: "d",
        bit: 3,
        help: Some("decimal-mode flag"),
    },
    inspect::FlagName {
        name: "i",
        bit: 2,
        help: Some("interrupt-disable flag"),
    },
    inspect::FlagName {
        name: "z",
        bit: 1,
        help: Some("zero flag — set when a result is zero"),
    },
    inspect::FlagName {
        name: "c",
        bit: 0,
        help: Some("carry flag — set on carry or borrow"),
    },
];

/// Bounds a syncless kernel: ~20 NTSC frames of minimum-length instructions.
const FRAME_INSTRUCTION_BUDGET: u32 = 200_000;

/// The 6507 register file as one inspection group. Shared by the live debugger
/// and the running snapshot so both produce identical groups.
pub fn cpu_register_groups(
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    s: u8,
    p: u8,
) -> Vec<inspect::RegisterGroup> {
    let hex = |name, value: u32, bits| inspect::Register {
        name,
        value,
        bits,
        style: inspect::ValueStyle::Hex,
        help: None,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("pc", pc as u32, 16).help("program counter"),
            hex("a", a as u32, 8).help("accumulator"),
            hex("x", x as u32, 8).help("X index register"),
            hex("y", y as u32, 8).help("Y index register"),
            hex("s", s as u32, 8).help("stack pointer (offset into page 1)"),
            inspect::Register {
                name: "p",
                value: p as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(MOS6502_FLAGS),
                help: Some("processor status flags"),
            },
        ],
    }]
}

pub struct Debugger {
    vcs: Vcs,
    breakpoints: BTreeSet<u16>,
}

/// Why a stepping call returned.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum Stop {
    Completed,
    Breakpoint,
    BudgetExhausted,
}

impl Debugger {
    pub fn new(vcs: Vcs) -> Self {
        Debugger {
            vcs,
            breakpoints: BTreeSet::new(),
        }
    }

    pub fn console(&self) -> &Vcs {
        &self.vcs
    }

    pub fn console_mut(&mut self) -> &mut Vcs {
        &mut self.vcs
    }

    pub fn into_console(self) -> Vcs {
        self.vcs
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    pub fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    /// The 6507 drives 13 address lines: breakpoints compare on them.
    fn at_breakpoint(&self) -> bool {
        self.breakpoints
            .iter()
            .any(|&bp| bp & 0x1FFF == self.vcs.cpu.pc & 0x1FFF)
    }

    /// Execute one instruction; a frame completing mid-instruction
    /// surfaces here.
    pub fn step(&mut self) -> Option<Frame> {
        self.vcs.step_instruction();
        self.vcs.take_frame()
    }

    /// Like step, but a JSR runs to the instruction after the call
    /// (bounded, and stopping at breakpoints inside the subroutine).
    pub fn step_over(&mut self) -> (Option<Frame>, Stop) {
        if self.vcs.peek(self.vcs.cpu.pc) != JSR {
            let frame = self.step();
            return (frame, Stop::Completed);
        }
        let return_address = self.vcs.cpu.pc.wrapping_add(3);
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            // Keep the newest frame completed while stepping.
            frame = self.vcs.take_frame().or(frame);
            if self.vcs.cpu.pc & 0x1FFF == return_address & 0x1FFF {
                return (frame, Stop::Completed);
            }
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
        }
        (frame, Stop::BudgetExhausted)
    }

    /// Run until the next frame completes or a breakpoint is hit.
    pub fn step_frame(&mut self) -> (Option<Frame>, Stop) {
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            if let Some(frame) = self.vcs.take_frame() {
                return (Some(frame), Stop::Completed);
            }
            if self.at_breakpoint() {
                return (None, Stop::Breakpoint);
            }
        }
        (None, Stop::BudgetExhausted)
    }

    /// The 6502 register file as one inspection group.
    pub fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        let cpu = &self.vcs.cpu;
        cpu_register_groups(cpu.pc, cpu.a, cpu.x, cpu.y, cpu.s, cpu.p)
    }

    /// The 6507's 13-line address map, named for what the board decodes, plus
    /// the board's bank-complete stores in the synthetic space above the bus: a
    /// `rom` region when the image exceeds the 4 KB window (a banked board), and
    /// `cart ram` when the board carries RAM the core stores accessibly.
    pub fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
        const fn region(name: &'static str, start: u32, len: u32) -> inspect::MemoryRegion {
            inspect::MemoryRegion { name, start, len }
        }
        let cartridge = self.vcs.cartridge();
        let mut regions = vec![
            region("tia", 0x0000, 0x40),
            region("riot-ram", 0x0080, 0x80),
            region("riot-io", 0x0280, 0x20),
            region("cartridge", 0x1000, 0x1000),
        ];
        let rom_len = cartridge.rom_len();
        if rom_len > 0x1000 {
            regions.push(region("rom", CART_ROM_BASE, rom_len as u32));
        }
        let ram_len = cartridge.ram_len();
        if ram_len > 0 {
            regions.push(region("cart ram", CART_RAM_BASE, ram_len as u32));
        }
        regions
    }

    /// Side-effect-free read of the 13-bit address space. Addresses in the
    /// synthetic bank-complete space read the board's raw ROM or RAM linearly,
    /// independent of the current bank; below it, the console bus.
    pub fn peek(&self, address: u32) -> u8 {
        let cartridge = self.vcs.cartridge();
        if address >= CART_ROM_BASE {
            cartridge.peek_rom((address - CART_ROM_BASE) as usize)
        } else if address >= CART_RAM_BASE {
            cartridge.peek_ram((address - CART_RAM_BASE) as usize)
        } else {
            self.vcs.peek(address as u16)
        }
    }

    pub fn pc(&self) -> u32 {
        self.vcs.cpu.pc as u32
    }

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Mos6502
    }

    /// Run until a breakpoint (or budget); frames surface as they complete.
    pub fn run(&mut self) -> (Option<Frame>, Stop) {
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            // Keep the newest frame completed while stepping.
            frame = self.vcs.take_frame().or(frame);
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
        }
        (frame, Stop::BudgetExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TvStandard;
    use crate::cartridge::CartType;

    /// Each 4 KB window-sized chunk filled with its bank index, so a linear read
    /// reveals which bank a byte came from.
    fn bank_stamped(size: usize) -> Vec<u8> {
        let mut rom = vec![0u8; size];
        for (i, bank) in rom.chunks_mut(0x1000).enumerate() {
            bank.fill(i as u8);
        }
        rom
    }

    fn debugger(rom: &[u8], cart_type: CartType) -> Debugger {
        Debugger::new(Vcs::new(rom, TvStandard::Ntsc, Some(cart_type)).expect("valid image"))
    }

    #[test]
    fn rom_region_only_for_banked_boards() {
        let banked = debugger(&bank_stamped(0x2000), CartType::F8);
        let rom = banked
            .memory_regions()
            .into_iter()
            .find(|r| r.name == "rom")
            .expect("banked board exposes a rom region");
        assert_eq!(rom.start, CART_ROM_BASE);
        assert_eq!(rom.len, 0x2000);

        // A 4 KB plain board is fully visible through the window already.
        let plain = debugger(&vec![0u8; 0x1000], CartType::Plain4K);
        assert!(plain.memory_regions().iter().all(|r| r.name != "rom"));
    }

    #[test]
    fn cart_ram_region_for_superchip() {
        let sc = debugger(&vec![0u8; 0x2000], CartType::F8Sc);
        let ram = sc
            .memory_regions()
            .into_iter()
            .find(|r| r.name == "cart ram")
            .expect("Superchip board exposes a cart-ram region");
        assert_eq!(ram.start, CART_RAM_BASE);
        assert_eq!(ram.len, 0x80);

        let plain = debugger(&vec![0u8; 0x1000], CartType::Plain4K);
        assert!(plain.memory_regions().iter().all(|r| r.name != "cart ram"));
    }

    #[test]
    fn synthetic_rom_peek_reads_unmapped_bank() {
        let banked = debugger(&bank_stamped(0x2000), CartType::F8);
        // File order, independent of the currently paged bank.
        assert_eq!(banked.peek(CART_ROM_BASE), 0);
        assert_eq!(banked.peek(CART_ROM_BASE + 0x1000), 1);
    }
}
