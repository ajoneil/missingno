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
pub mod instructions;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::symbols;

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
    BusRead { address: u16 },
    BusWrite { address: u16 },
    DmaRead { address: u16 },
    DmaWrite { address: u16 },
    Scanline(u8),
    PpuMode(Mode),
    PixelCounter(u8),
    PpuRegister { register: ppu::Register, value: u8 },
    CpuRegister { register: CpuRegister, value: u8 },
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

/// Named bits of the SM83 flags register `f`.
const SM83_FLAGS: &[inspect::FlagName] = &[
    inspect::FlagName { name: "z", bit: 7 },
    inspect::FlagName { name: "n", bit: 6 },
    inspect::FlagName { name: "h", bit: 5 },
    inspect::FlagName { name: "c", bit: 4 },
];

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

static WATCHABLES: &[WatchableSpec] = &[
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
        let cpu = self.game_boy.cpu();
        let hex8 = |name, value: u8| inspect::Register {
            name,
            value: value as u32,
            bits: 8,
            style: inspect::ValueStyle::Hex,
        };
        let hex16 = |name, value: u16| inspect::Register {
            name,
            value: value as u32,
            bits: 16,
            style: inspect::ValueStyle::Hex,
        };
        vec![inspect::RegisterGroup {
            name: "cpu",
            registers: vec![
                hex8("a", cpu.a),
                inspect::Register {
                    name: "f",
                    value: cpu.flags.bits() as u32,
                    bits: 8,
                    style: inspect::ValueStyle::Flags(SM83_FLAGS),
                },
                hex8("b", cpu.b),
                hex8("c", cpu.c),
                hex8("d", cpu.d),
                hex8("e", cpu.e),
                hex8("h", cpu.h),
                hex8("l", cpu.l),
                hex16("sp", cpu.stack_pointer),
                hex16("pc", cpu.pc),
            ],
        }]
    }

    /// The CPU-visible flat address map, named by role.
    pub fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
        let region = |name, start, len| inspect::MemoryRegion { name, start, len };
        vec![
            region("rom0", 0x0000, 0x4000),
            region("romx", 0x4000, 0x4000),
            region("vram", 0x8000, 0x2000),
            region("extram", 0xA000, 0x2000),
            region("wram", 0xC000, 0x2000),
            region("oam", 0xFE00, 0xA0),
            region("io", 0xFF00, 0x80),
            region("hram", 0xFF80, 0x7F),
        ]
    }

    /// Side-effect-free read of the CPU address space.
    pub fn peek(&self, address: u32) -> u8 {
        self.game_boy.peek(address as u16)
    }

    /// The address the debugger keys instructions on — the current opcode's
    /// fetch address, held for the whole instruction.
    pub fn pc(&self) -> u32 {
        self.game_boy.cpu().ir_address as u32
    }

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Sm83
    }

    pub fn watchables(&self) -> Vec<inspect::Watchable> {
        WATCHABLES
            .iter()
            .map(|spec| inspect::Watchable {
                key: spec.key,
                label: spec.label,
                param: spec.param,
            })
            .collect()
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
