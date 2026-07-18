//! A generic disassembly pane, registered for every family. It renders at the
//! Game Boy Instructions pane's standard from the system seam alone: the walk
//! and decode come from the core's [`InstructionSet`], the memory from
//! side-effect-free peeks (paused) or the per-vblank snapshot window (running),
//! and every row draws through the shared [`disasm_rows`] widgets so the two
//! panes stay pixel-identical. A core with no instruction set falls back to raw
//! byte rows.

use iced::{
    Length,
    widget::{Column, pane_grid, text},
};

use missingno_core::cdl::CdlWindow;
use missingno_core::disasm::{
    ReadMemory, Row, addresses_before, logged_addresses_before, window_after,
};
use missingno_core::isa::InstructionSet;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{InspectSnapshot, SystemDebugger};

use crate::app::{
    self,
    debugger::{
        disasm_rows,
        panes::{self, DebuggerPane, Pane, PaneContext, pane, running_placeholder},
    },
    ui::{fonts, palette},
};

/// Instructions of backward context shown above the current PC.
const CONTEXT_BEFORE: usize = 4;
/// Rows shown from the current PC onwards.
const CONTEXT_AFTER: usize = 80;

/// One built disassembly row, decoded once at context-build time so the pane
/// renders without re-consulting the core.
pub enum DisasmRow {
    Label(String),
    Data {
        address: u32,
        bank: Option<u16>,
        byte: u8,
    },
    Instruction {
        address: u32,
        bank: Option<u16>,
        tokens: Vec<disasm_rows::Token>,
        is_current: bool,
    },
    /// A raw byte for a core with no instruction set.
    Byte {
        address: u32,
        bank: Option<u16>,
        byte: u8,
        is_current: bool,
    },
}

/// The owned disassembly the context builder holds so the pane can borrow it.
pub struct DisasmReadout {
    pub rows: Vec<DisasmRow>,
}

/// The borrowed rows the pane renders from.
#[derive(Clone, Copy)]
pub struct DisasmPaneData<'b> {
    pub rows: &'b [DisasmRow],
}

impl<'b> DisasmPaneData<'b> {
    pub fn new(readout: &'b DisasmReadout) -> Self {
        Self {
            rows: &readout.rows,
        }
    }
}

/// Present a side-effect-free peek as the address walker's memory.
struct PeekMemory<'a>(&'a dyn SystemDebugger);

impl ReadMemory for PeekMemory<'_> {
    fn read(&self, address: u32) -> u8 {
        self.0.peek(address)
    }
}

/// Present the snapshot's captured window as the walker's memory; a walk off
/// the window's bounds reads open bus so it can't run past what was captured.
struct WindowMemory<'a>(&'a missingno_core::inspect::MemoryWindow);

impl ReadMemory for WindowMemory<'_> {
    fn read(&self, address: u32) -> u8 {
        self.0.read(address).unwrap_or(0xFF)
    }
}

/// Build the paused readout from the live core.
pub fn paused_readout(core: &dyn SystemDebugger) -> DisasmReadout {
    let cdl = core.cdl_window();
    let symbols = core.symbols();
    build(
        core.instruction_set(),
        core.pc(),
        &PeekMemory(core),
        Some(&cdl),
        Some(&symbols),
        &|address| core.bank_for(address),
    )
}

/// Build the running readout from a per-vblank snapshot; `None` when the
/// snapshot carries no program counter or memory window to walk.
pub fn running_readout(snapshot: &dyn InspectSnapshot) -> Option<DisasmReadout> {
    let pc = snapshot.pc()?;
    let window = snapshot.memory_window()?;
    Some(build(
        snapshot.instruction_set(),
        pc,
        &WindowMemory(window),
        snapshot.cdl_window(),
        snapshot.symbols(),
        &|address| snapshot.bank_for(address),
    ))
}

fn build(
    isa: Option<&dyn InstructionSet>,
    pc: u32,
    memory: &dyn ReadMemory,
    cdl: Option<&CdlWindow>,
    symbols: Option<&SymbolTable>,
    bank_of: &dyn Fn(u32) -> Option<u16>,
) -> DisasmReadout {
    let Some(isa) = isa else {
        return byte_fallback(pc, memory, bank_of);
    };

    let mut rows = Vec::new();
    let push_label = |rows: &mut Vec<DisasmRow>, address: u32| {
        if let Some(label) = symbols.and_then(|s| s.label_at(address as u16, bank_of(address))) {
            rows.push(DisasmRow::Label(label.to_owned()));
        }
    };

    // Backward context: exact where the log has coverage, heuristic otherwise.
    let before = match cdl {
        Some(cdl) => logged_addresses_before(pc, CONTEXT_BEFORE, isa, memory, cdl)
            .unwrap_or_else(|| addresses_before(pc, CONTEXT_BEFORE, isa, memory)),
        None => addresses_before(pc, CONTEXT_BEFORE, isa, memory),
    };
    for address in before {
        push_label(&mut rows, address);
        rows.push(decode_row(isa, memory, address, bank_of(address), false));
    }

    for row in window_after(pc, CONTEXT_AFTER, isa, memory, cdl) {
        match row {
            Row::Data(address) => {
                push_label(&mut rows, address);
                rows.push(DisasmRow::Data {
                    address,
                    bank: bank_of(address),
                    byte: memory.read(address),
                });
            }
            Row::Instruction(address) => {
                push_label(&mut rows, address);
                rows.push(decode_row(
                    isa,
                    memory,
                    address,
                    bank_of(address),
                    address == pc,
                ));
            }
        }
    }

    DisasmReadout { rows }
}

fn decode_row(
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
    address: u32,
    bank: Option<u16>,
    is_current: bool,
) -> DisasmRow {
    let mask = isa.address_mask();
    let bytes: Vec<u8> = (0..isa.max_len())
        .map(|i| memory.read(address.wrapping_add(i as u32) & mask))
        .collect();
    let decoded = isa.decode(address, &bytes);
    DisasmRow::Instruction {
        address,
        bank,
        tokens: disasm_rows::tokenize(isa, &decoded.mnemonic),
        is_current,
    }
}

/// Raw byte rows around the program counter, for a core with no instruction
/// set. Wraps in the 16-bit space these cores address.
fn byte_fallback(
    pc: u32,
    memory: &dyn ReadMemory,
    bank_of: &dyn Fn(u32) -> Option<u16>,
) -> DisasmReadout {
    const MASK: u32 = 0xFFFF;
    let rows = (0..(CONTEXT_BEFORE + CONTEXT_AFTER))
        .map(|i| {
            let address = pc
                .wrapping_add(i as u32)
                .wrapping_sub(CONTEXT_BEFORE as u32)
                & MASK;
            DisasmRow::Byte {
                address,
                bank: bank_of(address),
                byte: memory.read(address),
                is_current: address == pc,
            }
        })
        .collect();
    DisasmReadout { rows }
}

pub struct DisassemblyPane;

impl DisassemblyPane {
    pub fn new() -> Self {
        Self
    }
}

impl Pane for DisassemblyPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Disassembly
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(ctx) = ctx else {
            return running_placeholder("Disassembly");
        };
        let Some(data) = ctx.disasm else {
            return running_placeholder("Disassembly");
        };

        let breakpoints = ctx.breakpoints;
        let rows: Vec<_> = data
            .rows
            .iter()
            .map(|row| match row {
                DisasmRow::Label(label) => disasm_rows::label_row(label),
                DisasmRow::Data {
                    address,
                    bank,
                    byte,
                } => disasm_rows::data_row(*address, *bank, *byte),
                DisasmRow::Instruction {
                    address,
                    bank,
                    tokens,
                    is_current,
                } => disasm_rows::instruction_row(
                    *address,
                    *bank,
                    tokens,
                    *is_current,
                    breakpoints.contains(address),
                ),
                DisasmRow::Byte {
                    address,
                    bank,
                    byte,
                    is_current,
                } => disasm_rows::byte_row(
                    *address,
                    *bank,
                    *byte,
                    *is_current,
                    breakpoints.contains(address),
                ),
            })
            .collect();

        let header = if breakpoints.is_empty() {
            panes::title_bar("Disassembly")
        } else {
            panes::title_bar_with_detail(
                "Disassembly",
                text(format!("{} bp", breakpoints.len()))
                    .font(fonts::monospace())
                    .size(11.0)
                    .color(palette::MUTED),
            )
        };

        pane(
            header,
            iced::widget::scrollable(Column::from_vec(rows).width(Length::Fill))
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new()
                        .width(0)
                        .scroller_width(0),
                ))
                .width(Length::Fill)
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::cdl::{CODE, CdlWindow, DATA};
    use missingno_core::isa::{Flow, Instruction};

    /// A synthetic ISA whose instruction length is `opcode % 3 + 1`.
    struct Toy;
    impl InstructionSet for Toy {
        fn id(&self) -> &'static str {
            "toy"
        }
        fn max_len(&self) -> usize {
            3
        }
        fn decode(&self, _address: u32, bytes: &[u8]) -> Instruction {
            let opcode = bytes.first().copied().unwrap_or(0);
            Instruction {
                mnemonic: format!("op{opcode:02x}"),
                length: opcode % 3 + 1,
                flow: Flow::Sequential,
            }
        }
    }

    struct Bytes(Vec<u8>);
    impl ReadMemory for Bytes {
        fn read(&self, address: u32) -> u8 {
            self.0.get(address as usize).copied().unwrap_or(0)
        }
    }

    fn no_bank(_: u32) -> Option<u16> {
        None
    }

    #[test]
    fn marks_exactly_the_current_instruction() {
        let memory = Bytes(vec![0x00; 32]);
        let current: Vec<u32> = build(Some(&Toy), 8, &memory, None, None, &no_bank)
            .rows
            .iter()
            .filter_map(|row| match row {
                DisasmRow::Instruction {
                    address,
                    is_current: true,
                    ..
                } => Some(*address),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec![8]);
    }

    #[test]
    fn inserts_a_label_above_its_address() {
        let memory = Bytes(vec![0x00; 32]);
        let symbols = SymbolTable::parse("[labels]\n00:0008 Target\n");
        let readout = build(Some(&Toy), 8, &memory, None, Some(&symbols), &no_bank);
        let current = readout
            .rows
            .iter()
            .position(|row| {
                matches!(
                    row,
                    DisasmRow::Instruction {
                        is_current: true,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(matches!(&readout.rows[current - 1], DisasmRow::Label(l) if l == "Target"));
    }

    #[test]
    fn logged_data_becomes_a_data_row() {
        let memory = Bytes(vec![0x00; 8]);
        let cdl = CdlWindow::new(0, vec![CODE, DATA, CODE, CODE, CODE, CODE, CODE, CODE]);
        let readout = build(Some(&Toy), 0, &memory, Some(&cdl), None, &no_bank);
        assert!(
            readout
                .rows
                .iter()
                .any(|row| matches!(row, DisasmRow::Data { address: 1, .. }))
        );
    }

    #[test]
    fn backward_context_clamps_at_the_low_edge() {
        // From PC 0 there is nothing before it: the walk must not wrap or panic.
        let memory = Bytes(vec![0x00; 8]);
        let readout = build(Some(&Toy), 0, &memory, None, None, &no_bank);
        assert!(matches!(
            readout.rows.first(),
            Some(DisasmRow::Instruction {
                address: 0,
                is_current: true,
                ..
            })
        ));
    }

    #[test]
    fn byte_fallback_without_an_instruction_set() {
        let memory = Bytes((0..=255).collect());
        let readout = build(None, 10, &memory, None, None, &no_bank);
        assert!(
            readout
                .rows
                .iter()
                .all(|row| matches!(row, DisasmRow::Byte { .. }))
        );
        let current: Vec<u32> = readout
            .rows
            .iter()
            .filter_map(|row| match row {
                DisasmRow::Byte {
                    address,
                    is_current: true,
                    ..
                } => Some(*address),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec![10]);
    }

    #[test]
    fn applies_the_bank_prefix_in_the_switchable_region() {
        let memory = Bytes(vec![0x00; 0x8000]);
        let bank_of = |address: u32| (0x4000..0x8000).contains(&address).then_some(3);
        let bank = build(Some(&Toy), 0x4010, &memory, None, None, &bank_of)
            .rows
            .iter()
            .find_map(|row| match row {
                DisasmRow::Instruction {
                    is_current: true,
                    bank,
                    ..
                } => Some(*bank),
                _ => None,
            })
            .unwrap();
        assert_eq!(bank, Some(3));
    }
}
