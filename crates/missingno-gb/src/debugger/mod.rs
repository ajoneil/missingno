use std::collections::BTreeSet;
use std::path::Path;

use crate::{
    Console, Dmg, Model,
    cpu::instructions::Instruction,
    cpu_bus::{BusAccess, BusAccessKind},
    isa::Sm83,
    ppu::{self, rendering::Mode},
};
use cdl::CodeDataLog;
use instructions::InstructionsIterator;
use std::sync::Arc;
use symbols::SymbolTable;

pub mod cdl;
pub mod graphics;
pub mod inspection;
pub mod instructions;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::symbols;

// Synthetic address bases above the real bus, where the debugger exposes
// bank-complete cartridge stores past the CPU's bank-selected windows. Each
// gets its own decade with room for the largest image the family allows (GB ROM
// reaches 8 MB), and the same scheme is mirrored in the VCS debugger.
/// Bank-complete cartridge RAM, all banks linear.
const RAM_BASE: u32 = 0x0100_0000;
/// The full ROM image, all banks in file order.
const ROM_BASE: u32 = 0x0200_0000;
/// Bank-complete work RAM, all banks linear (CGB's eight banks; DMG has none).
const WRAM_BASE: u32 = 0x0300_0000;
/// Bank-complete video RAM, both banks linear (CGB's two banks; DMG has none).
const VRAM_BASE: u32 = 0x0400_0000;

/// Which bank-complete store a synthetic address resolves to.
#[derive(Clone, Copy)]
enum SyntheticStore {
    Rom,
    Sram,
    Wram,
    Vram,
}

/// Embedded profile for full T-cycle frame capture with all PPU details.
#[cfg(feature = "morepork")]
const FRAME_CAPTURE_PROFILE: &str = r#"
[profile]
name = "frame-capture"
description = "Full T-cycle trace with CPU registers, all PPU internals, timer, and interrupts."
trigger = "tcycle"

[fields]
cpu = "registers"
ppu = "all"
timer = true
interrupt = true
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpuRegister {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl std::fmt::Display for CpuRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            CpuRegister::A => "A",
            CpuRegister::B => "B",
            CpuRegister::C => "C",
            CpuRegister::D => "D",
            CpuRegister::E => "E",
            CpuRegister::H => "H",
            CpuRegister::L => "L",
        };
        f.write_str(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchCondition {
    BusRead {
        address: u16,
    },
    BusWrite {
        address: u16,
    },
    DmaRead {
        address: u16,
    },
    DmaWrite {
        address: u16,
    },
    Scanline(u8),
    PpuMode(Mode),
    PixelCounter(u8),
    PpuRegister {
        register: ppu::Register,
        value: u8,
    },
    CpuRegister {
        register: CpuRegister,
        value: u8,
    },
    /// The CPU reaches an instruction whose opcode fetch address is this — the
    /// same instruction-boundary point a plain breakpoint fires on.
    Pc(u16),
    /// The mapper pages this 16 KB bank into the `$4000` ROM window.
    RomBank(u16),
    /// The mapper pages this bank into the `$A000` cartridge-RAM window.
    SramBank(u8),
    /// The console pages this bank into the `$D000` work-RAM window (CGB).
    WramBank(u8),
    All(Vec<WatchCondition>),
}

impl std::fmt::Display for WatchCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchCondition::BusRead { address } => write!(f, "read {address:#06X}"),
            WatchCondition::BusWrite { address } => write!(f, "write {address:#06X}"),
            WatchCondition::DmaRead { address } => write!(f, "dma read {address:#06X}"),
            WatchCondition::DmaWrite { address } => write!(f, "dma write {address:#06X}"),
            WatchCondition::Scanline(line) => write!(f, "scanline {line}"),
            WatchCondition::PpuMode(mode) => write!(f, "mode {mode}"),
            WatchCondition::PixelCounter(counter) => write!(f, "pixel {counter}"),
            WatchCondition::PpuRegister { register, value } => {
                write!(f, "{register}={value:#04X}")
            }
            WatchCondition::CpuRegister { register, value } => {
                write!(f, "{register}={value:#04X}")
            }
            WatchCondition::Pc(address) => write!(f, "pc {address:#06X}"),
            WatchCondition::RomBank(bank) => write!(f, "rom-bank {bank}"),
            WatchCondition::SramBank(bank) => write!(f, "sram-bank {bank}"),
            WatchCondition::WramBank(bank) => write!(f, "wram-bank {bank}"),
            WatchCondition::All(conditions) => {
                for (i, condition) in conditions.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    write!(f, "{condition}")?;
                }
                Ok(())
            }
        }
    }
}

impl WatchCondition {
    fn needs_bus_trace(&self) -> bool {
        match self {
            WatchCondition::BusRead { .. }
            | WatchCondition::BusWrite { .. }
            | WatchCondition::DmaRead { .. }
            | WatchCondition::DmaWrite { .. } => true,
            WatchCondition::All(conditions) => conditions.iter().any(|c| c.needs_bus_trace()),
            _ => false,
        }
    }
}

/// The parameterised quantity behind one watchable key.
#[derive(Clone, PartialEq)]
enum WatchKind {
    BusRead,
    BusWrite,
    DmaRead,
    DmaWrite,
    Scanline,
    PixelCounter,
    PpuMode,
    PpuReg(ppu::Register),
    CpuReg(CpuRegister),
    Pc,
    RomBank,
    SramBank,
    WramBank,
}

/// A watchable key, its label and parameter shape, and the condition it maps
/// onto. The table is the single source for both directions so the exposed
/// keys and the `WatchCondition` mapping cannot drift apart.
struct WatchableSpec {
    key: &'static str,
    label: &'static str,
    param: inspect::WatchParam,
    kind: WatchKind,
}

const V8: inspect::WatchParam = inspect::WatchParam::Value { bits: 8 };
const V16: inspect::WatchParam = inspect::WatchParam::Value { bits: 16 };

/// Watchable keys the disassembly gutter composes into `{pc, bank}` watches on a
/// switchable-window row. Shared with `present_address` so a row's bank watch
/// and the exposed key cannot drift.
pub(crate) const ROM_BANK_KEY: &str = "rom-bank";
pub(crate) const SRAM_BANK_KEY: &str = "sram-bank";
pub(crate) const WRAM_BANK_KEY: &str = "wram-bank";

static WATCHABLES: &[WatchableSpec] = &[
    WatchableSpec {
        key: "pc",
        label: "PC",
        param: V16,
        kind: WatchKind::Pc,
    },
    WatchableSpec {
        key: ROM_BANK_KEY,
        label: "ROM bank",
        param: V16,
        kind: WatchKind::RomBank,
    },
    WatchableSpec {
        key: SRAM_BANK_KEY,
        label: "SRAM bank",
        param: V8,
        kind: WatchKind::SramBank,
    },
    WatchableSpec {
        key: WRAM_BANK_KEY,
        label: "WRAM bank",
        param: V8,
        kind: WatchKind::WramBank,
    },
    WatchableSpec {
        key: "bus-read",
        label: "Bus read",
        param: inspect::WatchParam::Address,
        kind: WatchKind::BusRead,
    },
    WatchableSpec {
        key: "bus-write",
        label: "Bus write",
        param: inspect::WatchParam::Address,
        kind: WatchKind::BusWrite,
    },
    WatchableSpec {
        key: "dma-read",
        label: "DMA read",
        param: inspect::WatchParam::Address,
        kind: WatchKind::DmaRead,
    },
    WatchableSpec {
        key: "dma-write",
        label: "DMA write",
        param: inspect::WatchParam::Address,
        kind: WatchKind::DmaWrite,
    },
    WatchableSpec {
        key: "scanline",
        label: "Scanline",
        param: V8,
        kind: WatchKind::Scanline,
    },
    WatchableSpec {
        key: "pixel-counter",
        label: "Pixel counter",
        param: V8,
        kind: WatchKind::PixelCounter,
    },
    WatchableSpec {
        key: "ppu-mode",
        label: "PPU mode",
        param: inspect::WatchParam::Value { bits: 2 },
        kind: WatchKind::PpuMode,
    },
    WatchableSpec {
        key: "ppu-lcdc",
        label: "LCDC",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::Control),
    },
    WatchableSpec {
        key: "ppu-stat",
        label: "STAT",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::Status),
    },
    WatchableSpec {
        key: "ppu-scy",
        label: "SCY",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::BackgroundViewportY),
    },
    WatchableSpec {
        key: "ppu-scx",
        label: "SCX",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::BackgroundViewportX),
    },
    WatchableSpec {
        key: "ppu-wy",
        label: "WY",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::WindowY),
    },
    WatchableSpec {
        key: "ppu-wx",
        label: "WX",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::WindowX),
    },
    WatchableSpec {
        key: "ppu-ly",
        label: "LY",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::CurrentScanline),
    },
    WatchableSpec {
        key: "ppu-lyc",
        label: "LYC",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::InterruptOnScanline),
    },
    WatchableSpec {
        key: "ppu-bgp",
        label: "BGP",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::BackgroundPalette),
    },
    WatchableSpec {
        key: "ppu-obp0",
        label: "OBP0",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::Sprite0Palette),
    },
    WatchableSpec {
        key: "ppu-obp1",
        label: "OBP1",
        param: V8,
        kind: WatchKind::PpuReg(ppu::Register::Sprite1Palette),
    },
    WatchableSpec {
        key: "cpu-a",
        label: "A",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::A),
    },
    WatchableSpec {
        key: "cpu-b",
        label: "B",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::B),
    },
    WatchableSpec {
        key: "cpu-c",
        label: "C",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::C),
    },
    WatchableSpec {
        key: "cpu-d",
        label: "D",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::D),
    },
    WatchableSpec {
        key: "cpu-e",
        label: "E",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::E),
    },
    WatchableSpec {
        key: "cpu-h",
        label: "H",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::H),
    },
    WatchableSpec {
        key: "cpu-l",
        label: "L",
        param: V8,
        kind: WatchKind::CpuReg(CpuRegister::L),
    },
];

fn mode_from_bits(value: u32) -> Option<Mode> {
    match value {
        0 => Some(Mode::HorizontalBlank),
        1 => Some(Mode::VerticalBlank),
        2 => Some(Mode::OamScan),
        3 => Some(Mode::Drawing),
        _ => None,
    }
}

/// Build the `WatchCondition` for one term, or `None` if the key is unknown or
/// a required parameter is missing.
fn condition_from_term(term: &inspect::WatchTerm) -> Option<WatchCondition> {
    let spec = WATCHABLES.iter().find(|s| s.key == term.key)?;
    let address = || term.address.map(|a| a as u16);
    let value = || term.value.map(|v| v as u8);
    let value16 = || term.value.map(|v| v as u16);
    Some(match &spec.kind {
        WatchKind::BusRead => WatchCondition::BusRead {
            address: address()?,
        },
        WatchKind::BusWrite => WatchCondition::BusWrite {
            address: address()?,
        },
        WatchKind::DmaRead => WatchCondition::DmaRead {
            address: address()?,
        },
        WatchKind::DmaWrite => WatchCondition::DmaWrite {
            address: address()?,
        },
        WatchKind::Scanline => WatchCondition::Scanline(value()?),
        WatchKind::PixelCounter => WatchCondition::PixelCounter(value()?),
        WatchKind::PpuMode => WatchCondition::PpuMode(mode_from_bits(term.value?)?),
        WatchKind::PpuReg(register) => WatchCondition::PpuRegister {
            register: *register,
            value: value()?,
        },
        WatchKind::CpuReg(register) => WatchCondition::CpuRegister {
            register: register.clone(),
            value: value()?,
        },
        WatchKind::Pc => WatchCondition::Pc(value16()?),
        WatchKind::RomBank => WatchCondition::RomBank(value16()?),
        WatchKind::SramBank => WatchCondition::SramBank(value()?),
        WatchKind::WramBank => WatchCondition::WramBank(value()?),
    })
}

/// The one term describing a non-compound condition; the key comes from the
/// same table `condition_from_term` reads.
fn term_from_condition(condition: &WatchCondition) -> inspect::WatchTerm {
    let (kind, address, value) = match condition {
        WatchCondition::BusRead { address } => (WatchKind::BusRead, Some(*address as u32), None),
        WatchCondition::BusWrite { address } => (WatchKind::BusWrite, Some(*address as u32), None),
        WatchCondition::DmaRead { address } => (WatchKind::DmaRead, Some(*address as u32), None),
        WatchCondition::DmaWrite { address } => (WatchKind::DmaWrite, Some(*address as u32), None),
        WatchCondition::Scanline(v) => (WatchKind::Scanline, None, Some(*v as u32)),
        WatchCondition::PixelCounter(v) => (WatchKind::PixelCounter, None, Some(*v as u32)),
        WatchCondition::PpuMode(m) => (WatchKind::PpuMode, None, Some(*m as u32)),
        WatchCondition::PpuRegister { register, value } => {
            (WatchKind::PpuReg(*register), None, Some(*value as u32))
        }
        WatchCondition::CpuRegister { register, value } => (
            WatchKind::CpuReg(register.clone()),
            None,
            Some(*value as u32),
        ),
        WatchCondition::Pc(address) => (WatchKind::Pc, None, Some(*address as u32)),
        WatchCondition::RomBank(bank) => (WatchKind::RomBank, None, Some(*bank as u32)),
        WatchCondition::SramBank(bank) => (WatchKind::SramBank, None, Some(*bank as u32)),
        WatchCondition::WramBank(bank) => (WatchKind::WramBank, None, Some(*bank as u32)),
        WatchCondition::All(_) => unreachable!("compounds are flattened before term conversion"),
    };
    let key = WATCHABLES
        .iter()
        .find(|s| s.kind == kind)
        .expect("every non-compound condition has a table entry")
        .key;
    inspect::WatchTerm {
        key: key.to_string(),
        address,
        value,
    }
}

/// Flatten a condition into terms, recursing through nested `All` compounds —
/// conjunction is associative, so a nested compound is the same set of terms.
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

/// A single term is its condition; several terms are their conjunction.
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

pub struct Debugger<M: Model = Dmg> {
    game_boy: Console<M>,
    breakpoints: BTreeSet<u16>,
    watchpoints: Vec<WatchCondition>,
    last_watchpoint_hit: Option<WatchCondition>,
    /// Labels from the ROM's `.sym` sidecar; shared so snapshots ride along.
    symbols: Arc<SymbolTable>,
    /// How each ROM byte has been used, filled in as the debugger runs.
    cdl: CodeDataLog,
    /// T-cycle counter. Increments once per dot. Not hardware state —
    /// debugging/tracing infrastructure built on top of the emulation core.
    tcycle_count: u64,
}

impl<M: Model> Debugger<M> {
    pub fn new(game_boy: Console<M>) -> Self {
        let cdl = CodeDataLog::new(game_boy.cartridge().rom_len());
        Self {
            game_boy,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_watchpoint_hit: None,
            symbols: Arc::new(SymbolTable::default()),
            cdl,
            tcycle_count: 0,
        }
    }

    pub fn cdl(&self) -> &CodeDataLog {
        &self.cdl
    }

    pub fn set_cdl(&mut self, cdl: CodeDataLog) {
        self.cdl = cdl;
    }

    pub fn set_symbols(&mut self, symbols: SymbolTable) {
        self.symbols = Arc::new(symbols);
    }

    pub fn symbols(&self) -> &Arc<SymbolTable> {
        &self.symbols
    }

    pub fn add_user_symbol(&mut self, symbol: symbols::Symbol) {
        Arc::make_mut(&mut self.symbols).add_user(symbol);
    }

    pub fn remove_user_symbol(&mut self, symbol: &symbols::Symbol) {
        Arc::make_mut(&mut self.symbols).remove_user(symbol);
    }

    pub fn save_symbols(&self, path: &Path) {
        self.symbols.save(path);
    }

    pub fn game_boy(&self) -> &Console<M> {
        &self.game_boy
    }

    pub fn game_boy_mut(&mut self) -> &mut Console<M> {
        &mut self.game_boy
    }

    pub fn game_boy_take(self) -> Console<M> {
        self.game_boy
    }

    pub fn tcycle_count(&self) -> u64 {
        self.tcycle_count
    }

    pub fn step(&mut self) -> Option<M::Screen> {
        let result = self.step_logged();
        self.tcycle_count += result.tcycles as u64;
        if result.new_screen {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    /// Step one instruction while logging its code/data usage into the CDL.
    fn step_logged(&mut self) -> crate::execute::StepResult {
        let before = self.game_boy.cpu().ir_address;
        let bank_before = self.game_boy.cartridge().switchable_rom_bank();
        let opcode = self.game_boy.read(before);
        let length = crate::cpu::instructions::instruction_length(opcode);

        let result = self.game_boy.step_recorded();

        self.cdl
            .mark(before, bank_before, cdl::CODE | cdl::INSTRUCTION_START);
        for offset in 1..length {
            self.cdl
                .mark(before.wrapping_add(offset), bank_before, cdl::CODE);
        }
        let instruction_end = before.wrapping_add(length);
        for access in self.game_boy.bus_trace() {
            let is_read = matches!(access.kind, BusAccessKind::Read | BusAccessKind::DmaRead);
            let in_instruction = access.address >= before && access.address < instruction_end;
            if is_read && !in_instruction {
                self.cdl.mark(access.address, bank_before, cdl::DATA);
            }
        }

        let after = self.game_boy.cpu().ir_address;
        if after != instruction_end {
            let bank_after = self.game_boy.cartridge().switchable_rom_bank();
            let to_subroutine =
                matches!(opcode, 0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc) || opcode & 0xc7 == 0xc7;
            let bits = if to_subroutine {
                cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT
            } else {
                cdl::JUMP_TARGET
            };
            self.cdl.mark(after, bank_after, bits);
        }
        result
    }

    pub fn step_tcycle(&mut self) -> Option<M::Screen> {
        self.tcycle_count += 1;
        if self.game_boy.step_tcycle() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_over(&mut self) -> Option<M::Screen> {
        let mut it = InstructionsIterator::new(self.game_boy.cpu().ir_address, &self.game_boy);
        Instruction::decode(&mut it);
        let next_address = it.address.unwrap();

        let temp_breakpoint = if self.breakpoints.contains(&next_address) {
            None
        } else {
            self.breakpoints.insert(next_address);
            Some(next_address)
        };

        let mut last_screen = None;

        loop {
            let screen = self.step_frame();
            match screen {
                Some(screen) => {
                    last_screen = Some(screen);
                }
                None => {
                    break;
                }
            }
        }

        if let Some(temp_breakpoint) = temp_breakpoint {
            self.breakpoints.remove(&temp_breakpoint);
        }

        last_screen
    }

    pub fn step_frame(&mut self) -> Option<M::Screen> {
        self.last_watchpoint_hit = None;
        if self.watchpoints.is_empty() {
            self.step_frame_simple()
        } else {
            self.step_frame_watched()
        }
    }

    fn step_frame_simple(&mut self) -> Option<M::Screen> {
        loop {
            let screen = self.step();
            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched(&mut self) -> Option<M::Screen> {
        if self.watchpoints.iter().any(|w| w.needs_bus_trace()) {
            self.step_frame_watched_traced()
        } else {
            self.step_frame_watched_dots()
        }
    }

    fn step_frame_watched_traced(&mut self) -> Option<M::Screen> {
        loop {
            let result = self.step_logged();
            self.tcycle_count += result.tcycles as u64;
            let screen = if result.new_screen {
                Some(self.game_boy.screen().clone())
            } else {
                None
            };

            let hit = self
                .watchpoints
                .iter()
                .find(|condition| self.condition_matches(condition, self.game_boy.bus_trace()))
                .cloned();
            if let Some(hit) = hit {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched_dots(&mut self) -> Option<M::Screen> {
        loop {
            let screen = self.step_tcycle();

            if let Some(hit) = self.check_watchpoints(&[]) {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn breakpoint_triggered(&self) -> bool {
        self.breakpoints.contains(&self.game_boy.cpu().ir_address)
    }

    fn check_watchpoints(&self, trace: &[BusAccess]) -> Option<WatchCondition> {
        for condition in &self.watchpoints {
            if self.condition_matches(condition, trace) {
                return Some(condition.clone());
            }
        }
        None
    }

    fn condition_matches(&self, condition: &WatchCondition, trace: &[BusAccess]) -> bool {
        let ppu = self.game_boy.ppu();
        let cpu = self.game_boy.cpu();

        match condition {
            WatchCondition::BusRead { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::Read && a.address == *address),
            WatchCondition::BusWrite { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::Write && a.address == *address),
            WatchCondition::DmaRead { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::DmaRead && a.address == *address),
            WatchCondition::DmaWrite { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::DmaWrite && a.address == *address),
            WatchCondition::Scanline(target) => {
                ppu.read_register(ppu::Register::CurrentScanline) == *target
            }
            WatchCondition::PpuMode(target) => ppu.mode() == *target,
            WatchCondition::PixelCounter(target) => ppu
                .pipeline_state()
                .is_some_and(|snap| snap.pixel_counter == *target),
            WatchCondition::PpuRegister { register, value } => {
                ppu.read_register(*register) == *value
            }
            WatchCondition::CpuRegister { register, value } => {
                let actual = match register {
                    CpuRegister::A => cpu.a,
                    CpuRegister::B => cpu.b,
                    CpuRegister::C => cpu.c,
                    CpuRegister::D => cpu.d,
                    CpuRegister::E => cpu.e,
                    CpuRegister::H => cpu.h,
                    CpuRegister::L => cpu.l,
                };
                actual == *value
            }
            WatchCondition::Pc(target) => cpu.ir_address == *target,
            WatchCondition::RomBank(target) => {
                self.game_boy.cartridge().switchable_rom_bank() == Some(*target)
            }
            WatchCondition::SramBank(target) => {
                self.game_boy.cartridge().mapped_ram_bank() == Some(*target)
            }
            WatchCondition::WramBank(target) => {
                self.game_boy.model().selected_wram_bank() == Some(*target)
            }
            WatchCondition::All(conditions) => {
                conditions.iter().all(|c| self.condition_matches(c, trace))
            }
        }
    }

    pub fn last_watchpoint_hit(&self) -> Option<&WatchCondition> {
        self.last_watchpoint_hit.as_ref()
    }

    /// The SM83 register file as one inspection group.
    pub fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        inspection::cpu_register_groups(self.game_boy.cpu())
    }

    /// The cartridge's bank-complete stores in the synthetic space above the
    /// bus, each bounded by its image length: the full ROM image, the cart RAM,
    /// and the banked work-RAM image (CGB). Shared by [`memory_regions`],
    /// [`peek`](Self::peek) and [`present_address`](Self::present_address) so the
    /// published bounds and the routing cannot drift. A store the cart lacks has
    /// length 0 and so contains no address.
    fn synthetic_regions(&self) -> [(SyntheticStore, inspect::MemoryRegion); 4] {
        let cartridge = self.game_boy.cartridge();
        let wram_len = self
            .game_boy
            .model()
            .wram_image()
            .map_or(0, |wram| wram.len() as u32);
        let vram_len = self.game_boy.vram_image_len().unwrap_or(0);
        let region = |name, start, len| inspect::MemoryRegion { name, start, len };
        [
            (
                SyntheticStore::Rom,
                region("rom", ROM_BASE, cartridge.rom_len() as u32),
            ),
            (
                SyntheticStore::Sram,
                region("sram", RAM_BASE, cartridge.ram_len() as u32),
            ),
            (
                SyntheticStore::Wram,
                region("wram-all", WRAM_BASE, wram_len),
            ),
            (
                SyntheticStore::Vram,
                region("vram-all", VRAM_BASE, vram_len),
            ),
        ]
    }

    /// The synthetic store `address` falls in and its linear offset within it,
    /// bounded by the region table — `None` for a bus address or one past every
    /// store.
    fn synthetic_route(&self, address: u32) -> Option<(SyntheticStore, u32)> {
        self.synthetic_regions()
            .into_iter()
            .find(|(_, region)| region.contains(address))
            .map(|(store, region)| (store, address - region.start))
    }

    /// The CPU-visible flat address map, named by role, plus the cartridge's
    /// bank-complete stores in the synthetic space above the bus: the full ROM
    /// image, `sram` when the cart has RAM, and `wram-all` on a console that
    /// banks work RAM (CGB). The bus-window regions (`rom0`/`romx`/`extram`) stay
    /// the CPU's bank-selected, enable-gated view.
    pub fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
        const fn region(name: &'static str, start: u32, len: u32) -> inspect::MemoryRegion {
            inspect::MemoryRegion { name, start, len }
        }
        let mut regions = vec![
            region("rom0", 0x0000, 0x4000),
            region("romx", 0x4000, 0x4000),
            region("vram", 0x8000, 0x2000),
            region("extram", 0xA000, 0x2000),
            region("wram", 0xC000, 0x2000),
            region("oam", 0xFE00, 0xA0),
            region("io", 0xFF00, 0x80),
            region("hram", 0xFF80, 0x7F),
        ];
        // The ROM image is always present; the RAM stores appear only when the
        // cart (or console) carries them.
        for (store, region) in self.synthetic_regions() {
            if matches!(store, SyntheticStore::Rom) || region.len > 0 {
                regions.push(region);
            }
        }
        regions
    }

    /// Side-effect-free read of the CPU address space. Addresses in the
    /// synthetic bank-complete space read the cart's raw ROM or RAM, or the
    /// banked work RAM, linearly — independent of the current bank; below it,
    /// the CPU bus. An address above the bus but past every store reads open bus.
    pub fn peek(&self, address: u32) -> u8 {
        let cartridge = self.game_boy.cartridge();
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => cartridge.peek_rom(offset as usize),
            Some((SyntheticStore::Sram, offset)) => cartridge.peek_ram(offset as usize),
            Some((SyntheticStore::Wram, offset)) => self
                .game_boy
                .model()
                .wram_image()
                .and_then(|wram| wram.get(offset as usize).copied())
                .unwrap_or(0xFF),
            Some((SyntheticStore::Vram, offset)) => self.game_boy.vram_image_byte(offset),
            None if address <= u16::MAX as u32 => self.game_boy.peek(address as u16),
            None => 0xFF,
        }
    }

    /// How `address` presents in the disassembly's address column: a synthetic
    /// bank-complete address as its bank and the CPU window it pages into (ROM
    /// bank 0 at `$0000`, banks ≥1 at `$4000`; SRAM at `$A000`; WRAM bank 0 at
    /// `$C000`, banks ≥1 at `$D000`), a plain bus address as itself. A
    /// breakpoint from a switchable-window row would fire for whichever bank is
    /// paged in, so only fixed-bank windows carry one.
    pub fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        use inspect::AddressDisplay;
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => {
                let bank = (offset / 0x4000) as u16;
                if bank == 0 {
                    AddressDisplay::fixed(offset, 0)
                } else {
                    AddressDisplay::banked(0x4000 + (offset % 0x4000), bank, ROM_BANK_KEY)
                }
            }
            Some((SyntheticStore::Sram, offset)) => AddressDisplay::banked(
                0xA000 + (offset % 0x2000),
                (offset / 0x2000) as u16,
                SRAM_BANK_KEY,
            ),
            Some((SyntheticStore::Wram, offset)) => {
                let bank = (offset / 0x1000) as u16;
                if bank == 0 {
                    AddressDisplay::fixed(0xC000 + offset, 0)
                } else {
                    AddressDisplay::banked(0xD000 + (offset % 0x1000), bank, WRAM_BANK_KEY)
                }
            }
            // Both VRAM banks page into the same $8000 window (VBK-switched);
            // there is no VBK bank watch, so the bank shows for orientation only.
            Some((SyntheticStore::Vram, offset)) => {
                AddressDisplay::shared_window(0x8000 + (offset % 0x2000), (offset / 0x2000) as u16)
            }
            None => {
                let bank = match address as u16 {
                    0x4000..=0x7FFF => self.game_boy.cartridge().switchable_rom_bank(),
                    _ => None,
                };
                AddressDisplay::bus(address, bank)
            }
        }
    }

    /// The synthetic bank-complete address whose row presents as `bank:window`,
    /// for jump-to-address — the inverse of [`present_address`](Self::present_address)
    /// over the synthetic space. `None` when no region carries that pairing. ROM
    /// bank 0 presents only through the fixed `$0000` window, so a `$4000`-window
    /// pairing with bank 0 has no synthetic row and is rejected.
    pub fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        let cartridge = self.game_boy.cartridge();
        let wram_len = self.game_boy.model().wram_image().map(<[u8]>::len);
        let vram_len = self.game_boy.vram_image_len();
        match window {
            0x0000..=0x3FFF if bank == 0 => {
                (window < cartridge.rom_len() as u32).then_some(ROM_BASE + window)
            }
            0x4000..=0x7FFF if bank != 0 => {
                let linear = bank as u32 * 0x4000 + (window - 0x4000);
                (linear < cartridge.rom_len() as u32).then_some(ROM_BASE + linear)
            }
            0xA000..=0xBFFF => {
                let linear = bank as u32 * 0x2000 + (window - 0xA000);
                (linear < cartridge.ram_len() as u32).then_some(RAM_BASE + linear)
            }
            0xC000..=0xCFFF if bank == 0 => {
                let linear = window - 0xC000;
                wram_len
                    .filter(|&len| (linear as usize) < len)
                    .map(|_| WRAM_BASE + linear)
            }
            0xD000..=0xDFFF => {
                let linear = bank as u32 * 0x1000 + (window - 0xD000);
                wram_len
                    .filter(|&len| (linear as usize) < len)
                    .map(|_| WRAM_BASE + linear)
            }
            0x8000..=0x9FFF => {
                let linear = bank as u32 * 0x2000 + (window - 0x8000);
                vram_len
                    .filter(|&len| linear < len)
                    .map(|_| VRAM_BASE + linear)
            }
            _ => None,
        }
    }

    /// The address the debugger keys instructions on — the current opcode's
    /// fetch address, held for the whole instruction.
    pub fn pc(&self) -> u32 {
        self.game_boy.cpu().ir_address as u32
    }

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Sm83
    }

    pub fn watchables(&self) -> &'static [inspect::Watchable] {
        use std::sync::OnceLock;
        // `wram-bank` is meaningful only on a console that banks work RAM (CGB);
        // a flat-WRAM console (DMG) never pages it, so it is dropped from the
        // list there. The two buckets are keyed by that capability, not by the
        // model type — a plain generic-fn static would be shared across all
        // monomorphizations.
        fn build(banks_wram: bool) -> Vec<inspect::Watchable> {
            WATCHABLES
                .iter()
                .filter(|spec| banks_wram || spec.key != WRAM_BANK_KEY)
                .map(|spec| inspect::Watchable {
                    key: spec.key,
                    label: spec.label,
                    param: spec.param,
                })
                .collect()
        }
        static WITH_WRAM: OnceLock<Vec<inspect::Watchable>> = OnceLock::new();
        static WITHOUT_WRAM: OnceLock<Vec<inspect::Watchable>> = OnceLock::new();
        if self.game_boy.model().wram_image().is_some() {
            WITH_WRAM.get_or_init(|| build(true))
        } else {
            WITHOUT_WRAM.get_or_init(|| build(false))
        }
    }

    pub fn add_watch(&mut self, watch: inspect::Watch) {
        if let Some(condition) = watch_to_condition(&watch) {
            self.add_watchpoint(condition);
        }
    }

    pub fn remove_watch(&mut self, watch: inspect::Watch) {
        if let Some(condition) = watch_to_condition(&watch) {
            self.remove_watchpoint(&condition);
        }
    }

    pub fn watches(&self) -> Vec<inspect::Watch> {
        self.watchpoints.iter().map(watch_from_condition).collect()
    }

    pub fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.last_watchpoint_hit.as_ref().map(watch_from_condition)
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
        self.tcycle_count = 0;
    }

    pub fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    pub fn watchpoints(&self) -> &[WatchCondition] {
        &self.watchpoints
    }

    pub fn add_watchpoint(&mut self, condition: WatchCondition) {
        if !self.watchpoints.contains(&condition) {
            self.watchpoints.push(condition);
        }
    }

    pub fn remove_watchpoint(&mut self, condition: &WatchCondition) {
        self.watchpoints.retain(|w| w != condition);
    }

    pub fn clear_watchpoints(&mut self) {
        self.watchpoints.clear();
    }

    /// Capture a full T-cycle trace of one frame to a .morepork file.
    #[cfg(feature = "morepork")]
    pub fn capture_frame(&mut self, path: impl AsRef<Path>) -> Result<M::Screen, String> {
        use crate::trace::{BootRom, Profile, Tracer};

        let profile = Profile::parse(FRAME_CAPTURE_PROFILE)
            .map_err(|e| format!("Failed to parse trace profile: {e}"))?;

        let mut tracer = Tracer::create(
            path,
            &profile,
            &self.game_boy,
            BootRom::Skip,
            M::TRACE_MODEL_NAME,
        )
        .map_err(|e| format!("Failed to create tracer: {e}"))?;

        // Mark frame boundary at entry 0 so all entries belong to this frame.
        tracer
            .mark_frame()
            .map_err(|e| format!("Trace mark_frame error: {e}"))?;

        let mut trace_err = None;
        loop {
            let mut frame = false;
            let mut is_first = true;
            self.game_boy.execute_tcycle_observed(|gb, result| {
                if let Some(pixel) = result.pixel {
                    match pixel.pixel {
                        ppu::TracePixel::Shade(shade) => tracer.push_pixel(shade),
                        ppu::TracePixel::Rgb555(color) => tracer.push_pixel_rgb555(color),
                    }
                }
                frame |= result.new_screen;
                if is_first {
                    is_first = false;
                } else if let Err(e) = tracer.capture(gb) {
                    trace_err = Some(format!("Trace capture error: {e}"));
                } else {
                    tracer.advance_dot();
                }
            });
            if let Some(e) = trace_err {
                return Err(e);
            }
            self.tcycle_count += 1;
            if frame {
                break;
            }
        }

        tracer
            .finish()
            .map_err(|e| format!("Failed to finish trace: {e}"))?;

        Ok(self.game_boy.screen().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOP; JP 0150 → CALL 0160 { LD A,42; LD A,(0200); RET } → JR self.
    fn traced_program_console() -> Console<Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        rom[0x150..0x153].copy_from_slice(&[0xcd, 0x60, 0x01]);
        rom[0x153..0x155].copy_from_slice(&[0x18, 0xfe]);
        rom[0x160..0x162].copy_from_slice(&[0x3e, 0x42]);
        rom[0x162..0x165].copy_from_slice(&[0xfa, 0x00, 0x02]);
        rom[0x165] = 0xc9;
        Console::new(crate::cartridge::Cartridge::new(rom, None), None)
    }

    #[test]
    fn stepping_logs_code_data_and_control_flow() {
        let mut debugger = Debugger::new(traced_program_console());
        for _ in 0..6 {
            debugger.step();
        }

        let flags = |address| debugger.cdl().flags(address, Some(1));
        assert_eq!(
            flags(0x0100) & (cdl::CODE | cdl::INSTRUCTION_START),
            cdl::CODE | cdl::INSTRUCTION_START
        );
        // JP operand byte: code, but not an instruction start.
        assert_eq!(flags(0x0103) & cdl::CODE, cdl::CODE);
        assert_eq!(flags(0x0103) & cdl::INSTRUCTION_START, 0);
        assert_eq!(
            flags(0x0150) & (cdl::CODE | cdl::JUMP_TARGET),
            cdl::CODE | cdl::JUMP_TARGET
        );
        assert_eq!(
            flags(0x0160) & (cdl::CODE | cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT),
            cdl::CODE | cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT
        );
        assert_eq!(flags(0x0200), cdl::DATA);
        assert_eq!(flags(0x0300), 0);
    }

    /// A four-bank MBC5 cart with 32 KB RAM, each ROM bank stamped with its
    /// index so a linear read reveals which bank a byte came from.
    fn mbc5_ram_cart() -> crate::cartridge::Cartridge {
        let mut rom = vec![0u8; 4 * 0x4000];
        for (i, bank) in rom.chunks_mut(0x4000).enumerate() {
            bank.fill(i as u8);
        }
        rom[0x147] = 0x1a; // MBC5 + RAM
        rom[0x149] = 3; // 32 KB (four 8 KB banks)
        crate::cartridge::Cartridge::new(rom, None)
    }

    #[test]
    fn sram_region_present_only_with_cart_ram() {
        let with_ram = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let regions = with_ram.memory_regions();
        let sram = regions.iter().find(|r| r.name == "sram").expect("sram");
        assert_eq!(sram.start, RAM_BASE);
        assert_eq!(sram.len, 4 * 0x2000);

        // traced_program_console is a plain no-RAM cart.
        let no_ram = Debugger::new(traced_program_console());
        assert!(no_ram.memory_regions().iter().all(|r| r.name != "sram"));
    }

    #[test]
    fn rom_region_spans_the_full_image() {
        let debugger = Debugger::new(traced_program_console());
        let rom = debugger
            .memory_regions()
            .into_iter()
            .find(|r| r.name == "rom")
            .expect("rom region");
        assert_eq!(rom.start, ROM_BASE);
        assert_eq!(rom.len, 0x8000);
    }

    #[test]
    fn synthetic_ram_peek_bypasses_bank_and_enable() {
        let mut cart = mbc5_ram_cart();
        cart.write(0x0000, 0x0A); // enable RAM
        cart.write(0x4000, 0x02); // RAM bank 2
        cart.write(0xA005, 0x77);
        cart.write(0x0000, 0x00); // disable RAM again
        let debugger = Debugger::new(Console::<Dmg>::new(cart, None));

        // The CPU bus sees the disabled RAM as open bus.
        assert_eq!(debugger.peek(0xA005), 0xFF);
        // The synthetic region reads the raw byte in bank 2 regardless.
        assert_eq!(debugger.peek(RAM_BASE + 2 * 0x2000 + 5), 0x77);
    }

    #[test]
    fn synthetic_rom_peek_reads_unmapped_bank() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        // File order, independent of what the mapper currently pages in.
        assert_eq!(debugger.peek(ROM_BASE), 0);
        assert_eq!(debugger.peek(ROM_BASE + 3 * 0x4000), 3);
    }

    #[test]
    fn present_and_locate_round_trip_over_synthetic_space() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let display = |a: u32| {
            let d = debugger.present_address(a);
            (d.bank, d.window, d.breakpoint)
        };
        // ROM bank 0 maps to the fixed $0000 window — an unambiguous breakpoint.
        assert_eq!(display(ROM_BASE + 0x0123), (Some(0), 0x0123, Some(0x0123)));
        assert_eq!(
            debugger.locate_bank_window(0, 0x0123),
            Some(ROM_BASE + 0x0123)
        );
        // ROM bank 3 maps to the switchable $4000 window — no breakpoint.
        let rom3 = 3 * 0x4000 + 0x0123;
        assert_eq!(display(ROM_BASE + rom3), (Some(3), 0x4123, None));
        assert_eq!(
            debugger.locate_bank_window(3, 0x4123),
            Some(ROM_BASE + rom3)
        );
        // SRAM bank 2 maps to the $A000 window.
        let sram2 = 2 * 0x2000 + 0x0055;
        assert_eq!(display(RAM_BASE + sram2), (Some(2), 0xA055, None));
        assert_eq!(
            debugger.locate_bank_window(2, 0xA055),
            Some(RAM_BASE + sram2)
        );
        // A pairing past the image, and a window in no synthetic region, reject.
        assert_eq!(debugger.locate_bank_window(99, 0x4000), None);
        assert_eq!(debugger.locate_bank_window(0, 0x8000), None);
        // ROM bank 0 presents only through the fixed $0000 window; a bank-0
        // pairing in the switchable $4000 window has no synthetic row.
        assert_eq!(debugger.locate_bank_window(0, 0x4123), None);
    }

    #[test]
    fn peek_past_a_synthetic_store_reads_open_bus() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let rom_len = debugger.game_boy().cartridge().rom_len() as u32;
        // One byte past the ROM image is in no store: open bus, not a truncated
        // re-read of the console bus.
        assert_eq!(debugger.peek(ROM_BASE + rom_len), 0xFF);
        // A synthetic address between the ROM and WRAM decades reads open bus.
        assert_eq!(debugger.peek(RAM_BASE + 0x00FF_FFFF), 0xFF);
    }

    /// A synthetic ROM anchor decodes the bytes of a bank the CPU bus does not
    /// currently page in — the whole point of the bank-complete space. The
    /// walk must preserve the anchor's high bits, not alias back onto the bus.
    #[test]
    fn synthetic_anchor_decodes_unmapped_bank_contents() {
        use missingno_core::disasm::{ReadMemory, Row, window_after};

        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        // The wake bank paged into $4000 is bank 1 (all 0x01); bank 3 is all
        // 0x03 and reachable only through the synthetic space.
        let anchor = ROM_BASE + 3 * 0x4000;
        assert_eq!(debugger.peek(anchor), 0x03);
        assert_eq!(debugger.peek(0x4000), 0x01);

        struct Peek<'a>(&'a Debugger<Dmg>);
        impl ReadMemory for Peek<'_> {
            fn read(&self, address: u32) -> u8 {
                self.0.peek(address)
            }
        }
        // 0x03 is INC BC (one byte), so rows step by one and carry the base.
        let rows = window_after(anchor, 3, &Sm83, &Peek(&debugger), None);
        assert_eq!(
            rows,
            vec![
                Row::Instruction(anchor),
                Row::Instruction(anchor + 1),
                Row::Instruction(anchor + 2),
            ]
        );
    }

    fn cart_with_ram(cart_type: u8, ram_size: u8) -> crate::cartridge::Cartridge {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = cart_type;
        rom[0x149] = ram_size;
        crate::cartridge::Cartridge::new(rom, None)
    }

    #[test]
    fn single_ram_bank_carts_match_sram_bank_zero() {
        // NoMbc+RAM, MBC2 and MBC7 carry exactly one RAM bank and no bank
        // register; a synthetic SRAM row composes a `{pc, sram-bank:0}` watch,
        // which must match while RAM is the mapped target rather than never fire.
        for (name, cart) in [
            ("NoMbc", cart_with_ram(0x08, 2)),
            ("MBC2", cart_with_ram(0x05, 0)),
            ("MBC7", cart_with_ram(0x22, 0)),
        ] {
            assert_eq!(cart.mapped_ram_bank(), Some(0), "{name}");
            let debugger = Debugger::new(Console::<Dmg>::new(cart, None));
            assert!(
                debugger.condition_matches(&WatchCondition::SramBank(0), &[]),
                "{name} sram-bank:0 watch should match"
            );
        }
    }

    #[test]
    fn mbc3_clock_mode_maps_no_ram_bank() {
        // MBC3 in RAM mode targets the selected bank; in clock mode it maps no
        // RAM, so the bank watch correctly does not match.
        let mut cart = cart_with_ram(0x10, 3);
        cart.write(0x4000, 0x02); // RAM mode, bank 2
        assert_eq!(cart.mapped_ram_bank(), Some(2));
        cart.write(0x4000, 0x08); // clock mode: seconds register
        assert_eq!(cart.mapped_ram_bank(), None);

        let debugger = Debugger::new(Console::<Dmg>::new(cart, None));
        assert!(!debugger.condition_matches(&WatchCondition::SramBank(0), &[]));
        assert!(!debugger.condition_matches(&WatchCondition::SramBank(2), &[]));
    }

    #[test]
    fn dmg_has_no_linear_wram_region() {
        let debugger = Debugger::new(traced_program_console());
        assert!(
            debugger
                .memory_regions()
                .iter()
                .all(|r| r.name != "wram-all")
        );
        // DMG's model exposes no bank-complete WRAM image.
        assert!(debugger.game_boy().model().wram_image().is_none());
    }

    #[test]
    fn dmg_has_no_linear_vram_region() {
        let debugger = Debugger::new(traced_program_console());
        assert!(
            debugger
                .memory_regions()
                .iter()
                .all(|r| r.name != "vram-all")
        );
        // DMG's single VRAM bank is fully visible through the $8000 window.
        assert!(debugger.game_boy().vram_image_len().is_none());
    }

    /// Code uploaded to work RAM and executed there disassembles live: the walk
    /// reads through peek and, since the code/data log never covers RAM, the
    /// backward context falls back to the heuristic sweep.
    #[test]
    fn disassembles_code_running_in_work_ram() {
        use missingno_core::disasm::{ReadMemory, Row, window_after};

        // Store a three-byte routine to $C000 (NOP; JR -3 → self-loop) then jump
        // to it, so the program counter ends up executing from WRAM.
        let mut rom = vec![0u8; 0x8000];
        let program = [
            0x3E, 0x00, // LD A,$00
            0xEA, 0x00, 0xC0, // LD ($C000),A
            0x3E, 0x18, // LD A,$18
            0xEA, 0x01, 0xC0, // LD ($C001),A
            0x3E, 0xFD, // LD A,$FD
            0xEA, 0x02, 0xC0, // LD ($C002),A
            0xC3, 0x00, 0xC0, // JP $C000
        ];
        rom[0x100..0x100 + program.len()].copy_from_slice(&program);
        let mut debugger = Debugger::new(Console::<Dmg>::new(
            crate::cartridge::Cartridge::new(rom, None),
            None,
        ));

        for _ in 0..64 {
            if debugger.pc() == 0xC000 {
                break;
            }
            debugger.step();
        }
        assert_eq!(debugger.pc(), 0xC000, "did not reach the WRAM routine");

        // The routine bytes are live in WRAM.
        assert_eq!(debugger.peek(0xC000), 0x00);
        assert_eq!(debugger.peek(0xC001), 0x18);
        assert_eq!(debugger.peek(0xC002), 0xFD);
        // The log records nothing for RAM addresses, so the disassembly's
        // backward context has no coverage and uses the heuristic.
        assert_eq!(debugger.cdl().flags(0xC000, None), 0);

        struct Peek<'a>(&'a Debugger<Dmg>);
        impl ReadMemory for Peek<'_> {
            fn read(&self, address: u32) -> u8 {
                self.0.peek(address)
            }
        }
        let cdl = debugger.cdl().window(0xC000, None);
        let rows = window_after(0xC000, 2, &Sm83, &Peek(&debugger), Some(&cdl));
        assert_eq!(
            rows,
            vec![Row::Instruction(0xC000), Row::Instruction(0xC001)]
        );
    }

    #[test]
    fn every_watchable_key_round_trips() {
        for spec in WATCHABLES {
            let (address, value) = match spec.param {
                inspect::WatchParam::Address => (Some(0x1234u32), None),
                inspect::WatchParam::Value { bits } => {
                    (None, Some(if bits >= 8 { 0x42 } else { 1 }))
                }
                inspect::WatchParam::AddressValue => (Some(0x1234), Some(0x42)),
                inspect::WatchParam::None => (None, None),
            };
            let watch = inspect::Watch::single(spec.key, address, value);
            let condition = watch_to_condition(&watch).expect("maps to a condition");
            let back = watch_from_condition(&condition);
            assert_eq!(watch, back, "key {} did not round-trip", spec.key);
        }
    }

    #[test]
    fn nested_all_flattens_to_terms() {
        let condition = WatchCondition::All(vec![
            WatchCondition::BusRead { address: 0x0100 },
            WatchCondition::All(vec![
                WatchCondition::CpuRegister {
                    register: CpuRegister::A,
                    value: 0x42,
                },
                WatchCondition::Scanline(0x10),
            ]),
        ]);
        let watch = watch_from_condition(&condition);
        assert_eq!(watch.terms.len(), 3);
        match watch_to_condition(&watch).expect("rebuilds") {
            WatchCondition::All(conditions) => assert_eq!(conditions.len(), 3),
            other => panic!("expected a compound, got {other}"),
        }
    }

    /// An MBC1 cart of eight 16 KB banks whose program selects `bank` into the
    /// `$4000` window and jumps there, where a self-loop parks the PC at `$4000`.
    fn mbc1_bank_jump(bank: u8) -> Console<Dmg> {
        let mut rom = vec![0u8; 8 * 0x4000];
        rom[0x147] = 0x01; // MBC1
        rom[0x148] = 0x04; // 128 KB
        rom[0x100..0x108].copy_from_slice(&[
            0x3e, bank, // LD A, bank
            0xea, 0x00, 0x20, // LD ($2000), A  — select ROM bank
            0xc3, 0x00, 0x40, // JP $4000
        ]);
        // Park the PC at $4000 in every bank so the jump lands on a self-loop.
        for b in 1..8 {
            rom[b * 0x4000..b * 0x4000 + 2].copy_from_slice(&[0x18, 0xfe]); // JR -2
        }
        Console::new(crate::cartridge::Cartridge::new(rom, None), None)
    }

    #[test]
    fn pc_watch_fires_at_the_breakpoint_instant() {
        // A pc watch stops where a plain breakpoint at the same address would:
        // both read the instruction-fetch address, so they land on the same row.
        let mut watched = Debugger::new(traced_program_console());
        watched.add_watch(inspect::Watch::single("pc", None, Some(0x0150)));
        watched.step_frame();
        assert_eq!(watched.pc(), 0x0150);
        assert!(watched.last_watch_hit().is_some());

        let mut broken = Debugger::new(traced_program_console());
        broken.set_breakpoint(0x0150);
        broken.step_frame();
        assert_eq!(broken.pc(), watched.pc());
    }

    #[test]
    fn compound_pc_bank_watch_gates_on_the_mapped_bank() {
        let compound = inspect::Watch {
            terms: vec![
                inspect::WatchTerm {
                    key: "pc".into(),
                    address: None,
                    value: Some(0x4000),
                },
                inspect::WatchTerm {
                    key: "rom-bank".into(),
                    address: None,
                    value: Some(3),
                },
            ],
        };

        // Bank 3 mapped: the PC reaches $4000 with the watched bank — it fires.
        let mut right = Debugger::new(mbc1_bank_jump(3));
        right.add_watch(compound.clone());
        right.step_frame();
        assert_eq!(right.pc(), 0x4000);
        assert!(right.last_watch_hit().is_some());

        // Bank 2 mapped: the PC still reaches $4000, but the bank term rejects,
        // so the watch never fires and the frame runs to completion.
        let mut wrong = Debugger::new(mbc1_bank_jump(2));
        wrong.add_watch(compound);
        wrong.step_frame();
        assert!(wrong.last_watch_hit().is_none());
    }

    #[test]
    fn watchables_expose_pc_and_bank_keys() {
        let debugger = Debugger::new(traced_program_console());
        let keys: Vec<&str> = debugger.watchables().iter().map(|w| w.key).collect();
        assert!(keys.contains(&"pc"));
        assert!(keys.contains(&"rom-bank"));
        assert!(keys.contains(&"sram-bank"));
        // A flat-WRAM console (DMG) does not page work RAM, so it hides the key.
        assert!(!keys.contains(&"wram-bank"));
    }

    #[test]
    fn step_over_runs_a_call_to_completion() {
        let mut debugger = Debugger::new(traced_program_console());
        debugger.step(); // NOP
        debugger.step(); // JP → at the CALL
        assert_eq!(debugger.game_boy().cpu().ir_address, 0x0150);
        debugger.step_over();
        assert_eq!(debugger.game_boy().cpu().ir_address, 0x0153);
        assert!(debugger.breakpoints().is_empty());
    }
}
