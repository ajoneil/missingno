use crate::Model;
use crate::cpu_bus::{BusAccess, BusAccessKind};
use crate::ppu::{self, rendering::Mode};
use missingno_core::inspect;

use super::Debugger;

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
    pub(super) fn needs_bus_trace(&self) -> bool {
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

pub(super) fn watch_from_condition(condition: &WatchCondition) -> inspect::Watch {
    let mut terms = Vec::new();
    flatten_terms(condition, &mut terms);
    inspect::Watch { terms }
}

/// The watch conditions a console can name. `wram-bank` is meaningful only on
/// one that banks work RAM (CGB); a flat-WRAM console (DMG) never pages it, so
/// the key is dropped there. The two buckets are keyed by that capability, not
/// by the model type — a plain generic-fn static would be shared across all
/// monomorphizations.
pub fn watchables(banks_work_ram: bool) -> &'static [inspect::Watchable] {
    use std::sync::OnceLock;
    fn build(banks_work_ram: bool) -> Vec<inspect::Watchable> {
        WATCHABLES
            .iter()
            .filter(|spec| banks_work_ram || spec.key != WRAM_BANK_KEY)
            .map(|spec| inspect::Watchable {
                key: spec.key,
                label: spec.label,
                param: spec.param,
            })
            .collect()
    }
    static WITH_WRAM: OnceLock<Vec<inspect::Watchable>> = OnceLock::new();
    static WITHOUT_WRAM: OnceLock<Vec<inspect::Watchable>> = OnceLock::new();
    if banks_work_ram {
        WITH_WRAM.get_or_init(|| build(true))
    } else {
        WITHOUT_WRAM.get_or_init(|| build(false))
    }
}

/// A single term is its condition; several terms are their conjunction.
pub(super) fn watch_to_condition(watch: &inspect::Watch) -> Option<WatchCondition> {
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

impl<M: Model> Debugger<M> {
    pub(super) fn check_watchpoints(&self, trace: &[BusAccess]) -> Option<WatchCondition> {
        for condition in &self.watchpoints {
            if self.condition_matches(condition, trace) {
                return Some(condition.clone());
            }
        }
        None
    }

    pub(super) fn condition_matches(
        &self,
        condition: &WatchCondition,
        trace: &[BusAccess],
    ) -> bool {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::tests::traced_program_console;
    use crate::{Console, Dmg};

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
}
