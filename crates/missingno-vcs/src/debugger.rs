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

/// The watchable key the disassembly gutter composes into a `{pc, cart-bank}`
/// watch on a banked-window row. Shared with `present_address` so a row's bank
/// watch and the exposed key cannot drift.
pub(crate) const CART_BANK_KEY: &str = "cart-bank";

/// A watch condition, evaluated at each instruction boundary — the same point a
/// plain PC breakpoint is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum WatchCondition {
    /// The CPU reaches this address (compared on the 13 decoded lines) — the
    /// instruction-boundary point a plain breakpoint fires on.
    Pc(u16),
    /// The board pages this 4 KB bank into the cart window. Reads nothing on an
    /// unbanked board, so it never matches there.
    CartBank(u16),
    /// A conjunction: every condition must hold.
    All(Vec<WatchCondition>),
}

/// A watchable key, its label and parameter shape, and the condition it maps
/// onto — the single source for both the exposed list and the mapping.
struct WatchableSpec {
    key: &'static str,
    label: &'static str,
    param: inspect::WatchParam,
}

static WATCHABLES: &[WatchableSpec] = &[
    // A full 16-bit address; the condition compares it on the 13 decoded lines.
    WatchableSpec {
        key: "pc",
        label: "PC",
        param: inspect::WatchParam::Value { bits: 16 },
    },
    WatchableSpec {
        key: CART_BANK_KEY,
        label: "cart bank",
        param: inspect::WatchParam::Value { bits: 16 },
    },
];

fn condition_from_term(term: &inspect::WatchTerm) -> Option<WatchCondition> {
    let value = term.value?;
    match term.key.as_str() {
        "pc" => Some(WatchCondition::Pc(value as u16)),
        key if key == CART_BANK_KEY => Some(WatchCondition::CartBank(value as u16)),
        _ => None,
    }
}

fn term_from_condition(condition: &WatchCondition) -> inspect::WatchTerm {
    let (key, value) = match condition {
        WatchCondition::Pc(address) => ("pc", *address as u32),
        WatchCondition::CartBank(bank) => (CART_BANK_KEY, *bank as u32),
        WatchCondition::All(_) => unreachable!("compounds are flattened before term conversion"),
    };
    inspect::WatchTerm {
        key: key.to_string(),
        address: None,
        value: Some(value),
    }
}

fn flatten_terms(condition: &WatchCondition, out: &mut Vec<inspect::WatchTerm>) {
    match condition {
        WatchCondition::All(conditions) => {
            for condition in conditions {
                flatten_terms(condition, out);
            }
        }
        leaf => out.push(term_from_condition(leaf)),
    }
}

fn watch_from_condition(condition: &WatchCondition) -> inspect::Watch {
    let mut terms = Vec::new();
    flatten_terms(condition, &mut terms);
    inspect::Watch { terms }
}

fn watch_to_condition(watch: &inspect::Watch) -> Option<WatchCondition> {
    let mut conditions = Vec::with_capacity(watch.terms.len());
    for term in &watch.terms {
        conditions.push(condition_from_term(term)?);
    }
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(WatchCondition::All(conditions)),
    }
}

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
    watchpoints: Vec<WatchCondition>,
    last_watchpoint_hit: Option<WatchCondition>,
}

/// Why a stepping call returned.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum Stop {
    Completed,
    Breakpoint,
    Watch,
    BudgetExhausted,
}

impl Debugger {
    pub fn new(vcs: Vcs) -> Self {
        Debugger {
            vcs,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_watchpoint_hit: None,
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

    fn condition_matches(&self, condition: &WatchCondition) -> bool {
        match condition {
            // The 6507 decodes 13 address lines: match on them, as breakpoints do.
            WatchCondition::Pc(target) => self.vcs.cpu.pc & 0x1FFF == target & 0x1FFF,
            WatchCondition::CartBank(target) => {
                self.vcs.cartridge().selected_bank() == Some(*target as usize)
            }
            WatchCondition::All(conditions) => conditions.iter().all(|c| self.condition_matches(c)),
        }
    }

    /// The first watch that holds at the current instruction boundary, if any.
    fn check_watchpoints(&self) -> Option<WatchCondition> {
        self.watchpoints
            .iter()
            .find(|condition| self.condition_matches(condition))
            .cloned()
    }

    pub fn watchables(&self) -> &'static [inspect::Watchable] {
        use std::sync::OnceLock;
        static PUBLIC: OnceLock<Vec<inspect::Watchable>> = OnceLock::new();
        PUBLIC.get_or_init(|| {
            WATCHABLES
                .iter()
                .map(|spec| inspect::Watchable {
                    key: spec.key,
                    label: spec.label,
                    param: spec.param,
                })
                .collect()
        })
    }

    pub fn add_watch(&mut self, watch: inspect::Watch) {
        if let Some(condition) = watch_to_condition(&watch)
            && !self.watchpoints.contains(&condition)
        {
            self.watchpoints.push(condition);
        }
    }

    pub fn remove_watch(&mut self, watch: &inspect::Watch) {
        if let Some(condition) = watch_to_condition(watch) {
            self.watchpoints.retain(|w| *w != condition);
        }
    }

    pub fn watches(&self) -> Vec<inspect::Watch> {
        self.watchpoints.iter().map(watch_from_condition).collect()
    }

    pub fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.last_watchpoint_hit.as_ref().map(watch_from_condition)
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
        self.last_watchpoint_hit = None;
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
            if let Some(hit) = self.check_watchpoints() {
                self.last_watchpoint_hit = Some(hit);
                return (frame, Stop::Watch);
            }
        }
        (frame, Stop::BudgetExhausted)
    }

    /// Run until the next frame completes, a breakpoint, or a watch.
    pub fn step_frame(&mut self) -> (Option<Frame>, Stop) {
        self.last_watchpoint_hit = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            if let Some(frame) = self.vcs.take_frame() {
                return (Some(frame), Stop::Completed);
            }
            if self.at_breakpoint() {
                return (None, Stop::Breakpoint);
            }
            if let Some(hit) = self.check_watchpoints() {
                self.last_watchpoint_hit = Some(hit);
                return (None, Stop::Watch);
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

    /// How `address` presents in the disassembly's address column: a synthetic
    /// bank-complete ROM address as its 4 KB bank and the `$F000` cart window it
    /// pages into, a plain bus address as itself. Banked boards page every bank
    /// through the same window, so a synthetic-row breakpoint would fire for
    /// whichever bank is selected — none is offered.
    pub fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        use inspect::AddressDisplay;
        if address >= CART_ROM_BASE {
            let linear = address - CART_ROM_BASE;
            AddressDisplay::banked(
                0xF000 + (linear % 0x1000),
                (linear / 0x1000) as u16,
                CART_BANK_KEY,
            )
        } else if address >= CART_RAM_BASE {
            let linear = address - CART_RAM_BASE;
            AddressDisplay::unmarked(0xF000 + (linear & 0x0FFF))
        } else {
            AddressDisplay::bus(address, None)
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

    /// Run until a breakpoint or watch (or budget); frames surface as they
    /// complete.
    pub fn run(&mut self) -> (Option<Frame>, Stop) {
        self.last_watchpoint_hit = None;
        let mut frame = None;
        for _ in 0..FRAME_INSTRUCTION_BUDGET {
            self.vcs.step_instruction();
            // Keep the newest frame completed while stepping.
            frame = self.vcs.take_frame().or(frame);
            if self.at_breakpoint() {
                return (frame, Stop::Breakpoint);
            }
            if let Some(hit) = self.check_watchpoints() {
                self.last_watchpoint_hit = Some(hit);
                return (frame, Stop::Watch);
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

    /// Write a 6507 reset vector pointing at `$F000` into a 4 KB bank image.
    fn reset_to_f000(bank: &mut [u8]) {
        bank[0xFFC] = 0x00;
        bank[0xFFD] = 0xF0;
    }

    fn pc_watch(address: u16) -> inspect::Watch {
        inspect::Watch::single("pc", None, Some(address as u32))
    }

    fn pc_bank_watch(address: u16, bank: u16) -> inspect::Watch {
        inspect::Watch {
            terms: vec![
                inspect::WatchTerm {
                    key: "pc".into(),
                    address: None,
                    value: Some(address as u32),
                },
                inspect::WatchTerm {
                    key: CART_BANK_KEY.into(),
                    address: None,
                    value: Some(bank as u32),
                },
            ],
        }
    }

    #[test]
    fn pc_watch_stops_at_the_address() {
        // NOP at $F000, then JMP $F001 self-loop; a pc watch at $F001 stops there.
        let mut rom = vec![0u8; 0x1000];
        rom[0x000..0x004].copy_from_slice(&[0xEA, 0x4C, 0x01, 0xF0]);
        reset_to_f000(&mut rom);
        let mut debugger = debugger(&rom, CartType::Plain4K);
        debugger.add_watch(pc_watch(0xF001));
        let (_, stop) = debugger.step_frame();
        assert_eq!(stop, Stop::Watch);
        assert_eq!(debugger.pc() & 0x1FFF, 0xF001 & 0x1FFF);
    }

    /// An F8 board (two 4 KB banks) whose identical banks run three NOPs, switch
    /// to bank 1 by touching the `$FFF9` hotspot, then self-loop at `$F006`.
    fn f8_bank_switch_rom() -> Vec<u8> {
        let mut bank = vec![0u8; 0x1000];
        bank[0x000..0x009].copy_from_slice(&[
            0xEA, 0xEA, 0xEA, // three NOPs → $F000..$F003
            0xAD, 0xF9, 0xFF, // LDA $FFF9 — hotspot, selects bank 1
            0x4C, 0x06, 0xF0, // JMP $F006 self-loop
        ]);
        reset_to_f000(&mut bank);
        [bank.clone(), bank].concat()
    }

    #[test]
    fn cart_bank_watch_gates_on_the_selected_bank() {
        // Reached before the hotspot: $F002 runs on the wake bank (0).
        let mut on_bank0 = debugger(&f8_bank_switch_rom(), CartType::F8);
        on_bank0.add_watch(pc_bank_watch(0xF002, 0));
        let (_, stop) = on_bank0.step_frame();
        assert_eq!(stop, Stop::Watch);
        assert_eq!(on_bank0.console().cartridge().selected_bank(), Some(0));

        // At the loop the board has switched to bank 1. A `{pc, cart-bank:0}`
        // watch is added first but must NOT match there; the `cart-bank:1` watch
        // does — proving the bank term gates the compound.
        let mut on_bank1 = debugger(&f8_bank_switch_rom(), CartType::F8);
        on_bank1.add_watch(pc_bank_watch(0xF006, 0));
        on_bank1.add_watch(pc_bank_watch(0xF006, 1));
        let (_, stop) = on_bank1.step_frame();
        assert_eq!(stop, Stop::Watch);
        assert_eq!(on_bank1.console().cartridge().selected_bank(), Some(1));
        let hit = on_bank1.last_watch_hit().expect("a watch fired");
        let bank_term = hit
            .terms
            .iter()
            .find(|t| t.key == CART_BANK_KEY)
            .expect("carries the bank term");
        assert_eq!(bank_term.value, Some(1));
    }

    #[test]
    fn watchables_expose_pc_and_cart_bank() {
        let debugger = debugger(&bank_stamped(0x2000), CartType::F8);
        let keys: Vec<&str> = debugger.watchables().iter().map(|w| w.key).collect();
        assert!(keys.contains(&"pc"));
        assert!(keys.contains(&CART_BANK_KEY));
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

    #[test]
    fn present_and_locate_round_trip_rom() {
        let banked = debugger(&bank_stamped(0x2000), CartType::F8);
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
