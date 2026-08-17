//! Debugging backend: instruction stepping with the seam's stops evaluated at
//! each boundary, and side-effect-free inspection through [`Vcs::peek`]. The
//! stores belong to the seam; this side translates them once per run and
//! compares them the way the board decodes.

mod watch;

#[cfg(test)]
pub(crate) mod test_support;

use std::collections::BTreeSet;

use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::machine::StopSet;
use missingno_mos_6502::Mos6502;

use crate::console::Vcs;
use watch::{WatchCondition, watch_from_condition, watch_to_condition};

pub(crate) use watch::CART_BANK_KEY;
pub use watch::{supports_watch, watchables};

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

/// Which bank-complete store a synthetic address resolves to.
#[derive(Clone, Copy)]
enum SyntheticStore {
    Rom,
    Ram,
}

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

/// The 6507 register file as one inspection group. Shared by the live debugger
/// and the running snapshot so both produce identical groups. The stack pointer
/// shows as the page-1 address it selects rather than as the raw `s` offset.
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
        purpose: None,
        active: None,
    };
    vec![inspect::RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("pc", pc as u32, 16)
                .help("program counter")
                .purpose(inspect::RegisterPurpose::ProgramCounter),
            hex("sp", 0x0100 | s as u32, 16)
                .help("stack pointer (offset into page 1)")
                .purpose(inspect::RegisterPurpose::StackPointer),
            hex("a", a as u32, 8).help("accumulator"),
            hex("x", x as u32, 8).help("X index register"),
            hex("y", y as u32, 8).help("Y index register"),
            inspect::Register {
                name: "p",
                value: p as u32,
                bits: 8,
                style: inspect::ValueStyle::Flags(MOS6502_FLAGS),
                help: Some("processor status flags"),
                purpose: None,
                active: None,
            },
        ],
    }]
}

pub struct Debugger {
    vcs: Vcs,
}

/// Why an instruction step stopped short of running clean.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Stop {
    Breakpoint,
    Watch(inspect::Watch),
}

/// The seam's stop stores in the form this side compares against: breakpoint
/// addresses and the watch terms translated into conditions. Built once per run,
/// so a per-instruction check costs no allocation.
pub struct Stops {
    breakpoints: BTreeSet<u16>,
    watchpoints: Vec<WatchCondition>,
}

impl Stops {
    pub fn new(stops: &StopSet) -> Self {
        Stops {
            breakpoints: stops.pc.iter().map(|&address| address as u16).collect(),
            watchpoints: stops
                .watches
                .iter()
                .filter_map(watch_to_condition)
                .collect(),
        }
    }
}

impl Debugger {
    pub fn new(vcs: Vcs) -> Self {
        Debugger { vcs }
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

    /// Execute one instruction, then evaluate the stops at the boundary it
    /// lands on — the point a PC breakpoint and a watch condition both fire.
    pub fn step(&mut self, stops: &Stops) -> Option<Stop> {
        self.vcs.step_instruction();
        self.check(stops)
    }

    /// The stop that holds at the current instruction boundary, if any; a
    /// breakpoint is reported ahead of a watch.
    fn check(&self, stops: &Stops) -> Option<Stop> {
        if stops.breakpoints.iter().any(|&bp| self.at_address(bp)) {
            return Some(Stop::Breakpoint);
        }
        let hit = stops
            .watchpoints
            .iter()
            .find(|condition| condition.matches(&self.vcs))?;
        Some(Stop::Watch(watch_from_condition(hit)))
    }

    /// Whether the program counter has reached `address`. The 6507 drives 13
    /// address lines: the comparison is on them.
    pub fn at_address(&self, address: u16) -> bool {
        self.vcs.cpu.pc & 0x1FFF == address & 0x1FFF
    }

    /// The address a call at the program counter returns to; `None` when the
    /// instruction there is not one.
    pub fn step_over_target(&self) -> Option<u16> {
        (self.vcs.peek(self.vcs.cpu.pc) == JSR).then(|| self.vcs.cpu.pc.wrapping_add(3))
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
        let mut regions = vec![
            region("tia", 0x0000, 0x40),
            region("riot-ram", 0x0080, 0x80),
            region("riot-io", 0x0280, 0x20),
            region("cartridge", 0x1000, 0x1000),
        ];
        let (rom, ram) = self.synthetic_regions();
        // A 4 KB board is fully visible through the window already; the ROM
        // image is advertised only when the board banks past it.
        if rom.len > 0x1000 {
            regions.push(rom);
        }
        if ram.len > 0 {
            regions.push(ram);
        }
        regions
    }

    /// The board's bank-complete stores in the synthetic space above the bus,
    /// each bounded by its image length: the full ROM image and the cart RAM.
    /// Shared with the routing so the published bounds and the routing cannot
    /// drift.
    fn synthetic_regions(&self) -> (inspect::MemoryRegion, inspect::MemoryRegion) {
        let cartridge = self.vcs.cartridge();
        let region = |name, start, len| inspect::MemoryRegion { name, start, len };
        (
            region("rom", CART_ROM_BASE, cartridge.rom_len() as u32),
            region("cart ram", CART_RAM_BASE, cartridge.ram_len() as u32),
        )
    }

    /// The synthetic store `address` falls in and its linear offset within it,
    /// bounded by the region table — `None` for a bus address or one past every
    /// store.
    fn synthetic_route(&self, address: u32) -> Option<(SyntheticStore, u32)> {
        let (rom, ram) = self.synthetic_regions();
        if rom.contains(address) {
            Some((SyntheticStore::Rom, address - rom.start))
        } else if ram.contains(address) {
            Some((SyntheticStore::Ram, address - ram.start))
        } else {
            None
        }
    }

    /// Side-effect-free read of the 13-bit address space. Addresses in the
    /// synthetic bank-complete space read the board's raw ROM or RAM linearly,
    /// independent of the current bank; below it, the console bus. An address
    /// above the bus but past every store reads open bus.
    pub fn peek(&self, address: u32) -> u8 {
        let cartridge = self.vcs.cartridge();
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => cartridge.peek_rom(offset as usize),
            Some((SyntheticStore::Ram, offset)) => cartridge.peek_ram(offset as usize),
            None if address <= u16::MAX as u32 => self.vcs.peek(address as u16),
            None => 0xFF,
        }
    }

    pub fn pc(&self) -> u32 {
        self.vcs.cpu.pc as u32
    }

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Mos6502
    }

    /// How `address` presents in the disassembly's address column: a synthetic
    /// bank-complete ROM address as its 4 KB bank and the `$F000` cart window it
    /// pages into, a plain bus address as itself. Banked boards page every bank
    /// through the same window, so a synthetic-row breakpoint would fire for
    /// whichever bank is selected — none is offered.
    pub fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        use inspect::AddressDisplay;
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => AddressDisplay::banked(
                0xF000 + (offset % 0x1000),
                (offset / 0x1000) as u16,
                CART_BANK_KEY,
            ),
            Some((SyntheticStore::Ram, offset)) => {
                AddressDisplay::unmarked(0xF000 + (offset & 0x0FFF))
            }
            None => AddressDisplay::bus(address, None),
        }
    }

    /// The synthetic ROM address whose row presents as `bank:window`, for
    /// jump-to-address — the inverse of [`present_address`](Self::present_address).
    /// The offset is the window's low 12 bits (the cart is mirrored wherever A12
    /// is set). `None` when the pairing lands past the image.
    pub fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        if window & 0x1000 == 0 {
            return None;
        }
        let linear = bank as u32 * 0x1000 + (window & 0x0FFF);
        (linear < self.vcs.cartridge().rom_len() as u32).then_some(CART_ROM_BASE + linear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::CartType;
    use test_support::{bank_stamped, debugger};

    #[test]
    fn rom_region_only_for_banked_boards() {
        let banked = debugger(&bank_stamped(0x2000), CartType::Atari8K);
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
        let sc = debugger(&vec![0u8; 0x2000], CartType::Atari8KSuperchip);
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
        let banked = debugger(&bank_stamped(0x2000), CartType::Atari8K);
        // File order, independent of the currently paged bank.
        assert_eq!(banked.peek(CART_ROM_BASE), 0);
        assert_eq!(banked.peek(CART_ROM_BASE + 0x1000), 1);
    }

    #[test]
    fn present_and_locate_round_trip_rom() {
        let banked = debugger(&bank_stamped(0x2000), CartType::Atari8K);
        let display = |a: u32| {
            let d = banked.present_address(a);
            (d.bank, d.window, d.breakpoint)
        };
        // Each 4 KB bank presents through the $F000 cart window; banked boards
        // page every bank there, so no synthetic-row breakpoint is offered.
        assert_eq!(display(CART_ROM_BASE + 0x1123), (Some(1), 0xF123, None));
        assert_eq!(
            banked.locate_bank_window(1, 0xF123),
            Some(CART_ROM_BASE + 0x1123)
        );
        assert_eq!(display(CART_ROM_BASE + 0x0055), (Some(0), 0xF055, None));
        // A window without A12 set is not a cart address; a bank past the image
        // rejects.
        assert_eq!(banked.locate_bank_window(0, 0x0055), None);
        assert_eq!(banked.locate_bank_window(9, 0xF000), None);
    }
}
