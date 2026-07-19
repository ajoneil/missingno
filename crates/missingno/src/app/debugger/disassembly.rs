//! A generic disassembly pane, registered for every family. It renders at the
//! Game Boy Instructions pane's standard from the system seam alone: the walk
//! and decode come from the core's [`InstructionSet`], the memory from
//! side-effect-free peeks (paused) or the per-vblank snapshot window (running),
//! and every row draws through the shared [`disasm_rows`] widgets so the two
//! panes stay pixel-identical. A core with no instruction set falls back to raw
//! byte rows.

use iced::{
    Length,
    widget::{Column, button, pane_grid, row, text, text_input},
};

use missingno_core::cdl::CdlWindow;
use missingno_core::disasm::{
    ReadMemory, Row, addresses_before, logged_addresses_before, window_after,
};
use missingno_core::inspect::AddressDisplay;
use missingno_core::isa::InstructionSet;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{InspectSnapshot, SystemDebugger};

use crate::app::{
    self,
    debugger::{
        self, disasm_rows,
        panes::{self, DebuggerPane, Pane, PaneContext, PaneMessage, pane, running_placeholder},
    },
    ui::{fonts, palette, sizes::s},
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
        display: AddressDisplay,
        byte: u8,
    },
    Instruction {
        display: AddressDisplay,
        tokens: Vec<disasm_rows::Token>,
        is_current: bool,
    },
    /// A raw byte for a core with no instruction set.
    Byte {
        display: AddressDisplay,
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

/// Build the paused readout from the live core, walking from `anchor` when the
/// pane has jumped somewhere, else from the program counter.
pub fn paused_readout(core: &dyn SystemDebugger, anchor: Option<u32>) -> DisasmReadout {
    let cdl = core.cdl_window();
    let symbols = core.symbols();
    build(
        core.instruction_set(),
        anchor.unwrap_or_else(|| core.pc()),
        core.pc(),
        &PeekMemory(core),
        Some(&cdl),
        Some(&symbols),
        &|address| core.present_address(address),
    )
}

/// Build the running readout from a per-vblank snapshot; `None` when the
/// snapshot carries no program counter or memory window to walk. Always
/// PC-anchored — the snapshot's window only covers the program counter.
pub fn running_readout(snapshot: &dyn InspectSnapshot) -> Option<DisasmReadout> {
    let pc = snapshot.pc()?;
    let window = snapshot.memory_window()?;
    Some(build(
        snapshot.instruction_set(),
        pc,
        pc,
        &WindowMemory(window),
        snapshot.cdl_window(),
        snapshot.symbols(),
        &|address| snapshot.present_address(address),
    ))
}

/// Walk `count` context around `anchor`, marking `pc` as the current row when
/// the walk crosses it (the anchor is the PC unless the pane jumped elsewhere).
fn build(
    isa: Option<&dyn InstructionSet>,
    anchor: u32,
    pc: u32,
    memory: &dyn ReadMemory,
    cdl: Option<&CdlWindow>,
    symbols: Option<&SymbolTable>,
    present: &dyn Fn(u32) -> AddressDisplay,
) -> DisasmReadout {
    let Some(isa) = isa else {
        return byte_fallback(anchor, pc, memory, present);
    };

    let mut rows = Vec::new();
    let push_label = |rows: &mut Vec<DisasmRow>, display: AddressDisplay| {
        if let Some(label) = symbols.and_then(|s| s.label_at(display.window as u16, display.bank)) {
            rows.push(DisasmRow::Label(label.to_owned()));
        }
    };

    // Backward context: exact where the log has coverage, heuristic otherwise.
    let before = match cdl {
        Some(cdl) => logged_addresses_before(anchor, CONTEXT_BEFORE, isa, memory, cdl)
            .unwrap_or_else(|| addresses_before(anchor, CONTEXT_BEFORE, isa, memory)),
        None => addresses_before(anchor, CONTEXT_BEFORE, isa, memory),
    };
    for address in before {
        let display = present(address);
        push_label(&mut rows, display);
        rows.push(decode_row(isa, memory, address, display, address == pc));
    }

    for row in window_after(anchor, CONTEXT_AFTER, isa, memory, cdl) {
        match row {
            Row::Data(address) => {
                let display = present(address);
                push_label(&mut rows, display);
                rows.push(DisasmRow::Data {
                    display,
                    byte: memory.read(address),
                });
            }
            Row::Instruction(address) => {
                let display = present(address);
                push_label(&mut rows, display);
                rows.push(decode_row(isa, memory, address, display, address == pc));
            }
        }
    }

    DisasmReadout { rows }
}

fn decode_row(
    isa: &dyn InstructionSet,
    memory: &dyn ReadMemory,
    address: u32,
    display: AddressDisplay,
    is_current: bool,
) -> DisasmRow {
    let mask = isa.address_mask();
    let bytes: Vec<u8> = (0..isa.max_len())
        .map(|i| memory.read(address.wrapping_add(i as u32) & mask))
        .collect();
    let decoded = isa.decode(address, &bytes);
    DisasmRow::Instruction {
        display,
        tokens: disasm_rows::tokenize(isa, &decoded.mnemonic),
        is_current,
    }
}

/// Raw byte rows around the anchor, for a core with no instruction set. Wraps in
/// the 16-bit space these cores address.
fn byte_fallback(
    anchor: u32,
    pc: u32,
    memory: &dyn ReadMemory,
    present: &dyn Fn(u32) -> AddressDisplay,
) -> DisasmReadout {
    const MASK: u32 = 0xFFFF;
    let rows = (0..(CONTEXT_BEFORE + CONTEXT_AFTER))
        .map(|i| {
            let address = anchor
                .wrapping_add(i as u32)
                .wrapping_sub(CONTEXT_BEFORE as u32)
                & MASK;
            DisasmRow::Byte {
                display: present(address),
                byte: memory.read(address),
                is_current: address == pc,
            }
        })
        .collect();
    DisasmReadout { rows }
}

/// A jump-to-address the pane emits for the debugger to resolve against the
/// live core (it owns the region/bank mapping); the resolved anchor comes back
/// as [`Message::SetAnchor`].
#[derive(Clone, Debug)]
pub enum Message {
    /// Text typed into the jump field.
    JumpInput(String),
    /// The resolved walk anchor: `Some` to jump there, `None` to follow the PC.
    SetAnchor(Option<u32>),
}

impl From<Message> for app::Message {
    fn from(val: Message) -> Self {
        panes::Message::Pane(PaneMessage::Disassembly(val)).into()
    }
}

pub struct DisassemblyPane {
    jump_input: String,
    /// A user-set walk anchor overriding the PC-follow default; `None` follows
    /// the program counter. Effective while paused — the running view is always
    /// PC-anchored (its snapshot window covers only the PC).
    anchor: Option<u32>,
}

impl DisassemblyPane {
    pub fn new() -> Self {
        Self {
            jump_input: String::new(),
            anchor: None,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::JumpInput(input) => self.jump_input = input,
            Message::SetAnchor(anchor) => {
                self.anchor = anchor;
                self.jump_input.clear();
            }
        }
    }

    /// The jump field and the button that returns to following the PC.
    fn controls(&self) -> iced::Element<'static, app::Message> {
        let jump = text_input("bank:addr", &self.jump_input)
            .font(fonts::monospace())
            .size(13.0)
            .width(Length::Fixed(110.0))
            .on_input(|value| Message::JumpInput(value).into())
            .on_submit(app::Message::Debugger(
                debugger::Message::ResolveDisasmJump(self.jump_input.clone()),
            ));

        let follow = {
            let label = text("PC").font(fonts::monospace()).size(13.0);
            let mut btn = button(label).style(button::text);
            if self.anchor.is_some() {
                btn = btn.on_press(Message::SetAnchor(None).into());
            }
            btn
        };

        row![jump, follow]
            .spacing(s())
            .padding([0.0, s()])
            .align_y(iced::alignment::Vertical::Center)
            .into()
    }
}

impl Pane for DisassemblyPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Disassembly
    }

    fn on_message(&mut self, message: &PaneMessage) {
        if let PaneMessage::Disassembly(message) = message {
            self.update(message.clone());
        }
    }

    fn disasm_anchor(&self) -> Option<u32> {
        self.anchor
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(ctx) = ctx else {
            return running_placeholder("Disassembly");
        };
        let Some(data) = ctx.disasm else {
            return running_placeholder("Disassembly");
        };

        let breakpoints = ctx.breakpoints;
        let is_breakpoint = |display: &AddressDisplay| {
            display
                .breakpoint
                .is_some_and(|bp| breakpoints.contains(&bp))
        };
        let rows: Vec<_> = data
            .rows
            .iter()
            .map(|row| match row {
                DisasmRow::Label(label) => disasm_rows::label_row(label),
                DisasmRow::Data { display, byte } => disasm_rows::data_row(*display, *byte),
                DisasmRow::Instruction {
                    display,
                    tokens,
                    is_current,
                } => disasm_rows::instruction_row(
                    *display,
                    tokens,
                    *is_current,
                    is_breakpoint(display),
                ),
                DisasmRow::Byte {
                    display,
                    byte,
                    is_current,
                } => disasm_rows::byte_row(*display, *byte, *is_current, is_breakpoint(display)),
            })
            .collect();

        // The anchor detail names where the walk starts when it isn't the PC.
        let detail = match self.anchor {
            Some(_) => Some(text("jumped").color(palette::YELLOW)),
            None if !breakpoints.is_empty() => {
                Some(text(format!("{} bp", breakpoints.len())).color(palette::MUTED))
            }
            None => None,
        }
        .map(|t| t.font(fonts::monospace()).size(11.0));

        let header = match detail {
            Some(detail) => panes::title_bar_with_detail("Disassembly", detail),
            None => panes::title_bar("Disassembly"),
        };

        let listing = iced::widget::scrollable(Column::from_vec(rows).width(Length::Fill))
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new()
                    .width(0)
                    .scroller_width(0),
            ))
            .width(Length::Fill)
            .height(Length::Fill);

        pane(
            header,
            Column::new()
                .push(self.controls())
                .push(listing)
                .spacing(s())
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

    /// A bus presentation: every address shows as itself, no bank prefix.
    fn bus(address: u32) -> AddressDisplay {
        AddressDisplay::bus(address, None)
    }

    #[test]
    fn marks_exactly_the_current_instruction() {
        let memory = Bytes(vec![0x00; 32]);
        let current: Vec<u32> = build(Some(&Toy), 8, 8, &memory, None, None, &bus)
            .rows
            .iter()
            .filter_map(|row| match row {
                DisasmRow::Instruction {
                    display,
                    is_current: true,
                    ..
                } => Some(display.window),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec![8]);
    }

    #[test]
    fn inserts_a_label_above_its_address() {
        let memory = Bytes(vec![0x00; 32]);
        let symbols = SymbolTable::parse("[labels]\n00:0008 Target\n");
        let readout = build(Some(&Toy), 8, 8, &memory, None, Some(&symbols), &bus);
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
        let readout = build(Some(&Toy), 0, 0, &memory, Some(&cdl), None, &bus);
        assert!(readout.rows.iter().any(|row| matches!(
            row,
            DisasmRow::Data {
                display: AddressDisplay { window: 1, .. },
                ..
            }
        )));
    }

    #[test]
    fn backward_context_clamps_at_the_low_edge() {
        // From PC 0 there is nothing before it: the walk must not wrap or panic.
        let memory = Bytes(vec![0x00; 8]);
        let readout = build(Some(&Toy), 0, 0, &memory, None, None, &bus);
        assert!(matches!(
            readout.rows.first(),
            Some(DisasmRow::Instruction {
                display: AddressDisplay { window: 0, .. },
                is_current: true,
                ..
            })
        ));
    }

    #[test]
    fn byte_fallback_without_an_instruction_set() {
        let memory = Bytes((0..=255).collect());
        let readout = build(None, 10, 10, &memory, None, None, &bus);
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
                    display,
                    is_current: true,
                    ..
                } => Some(display.window),
                _ => None,
            })
            .collect();
        assert_eq!(current, vec![10]);
    }

    #[test]
    fn presentation_supplies_the_bank_prefix_and_window() {
        // A synthetic ROM-style presentation: the walk address maps to a banked
        // window with a bank prefix.
        let memory = Bytes(vec![0x00; 0x8000]);
        let present = |address: u32| AddressDisplay {
            window: 0x4000 + (address & 0x3FFF),
            bank: Some(3),
            breakpoint: None,
        };
        let display = build(Some(&Toy), 0x0010, 0x0010, &memory, None, None, &present)
            .rows
            .iter()
            .find_map(|row| match row {
                DisasmRow::Instruction {
                    is_current: true,
                    display,
                    ..
                } => Some(*display),
                _ => None,
            })
            .unwrap();
        assert_eq!(display.bank, Some(3));
        assert_eq!(display.window, 0x4010);
        // A switchable-window row offers no breakpoint.
        assert_eq!(display.breakpoint, None);
    }

    #[test]
    fn anchor_walks_away_from_the_pc_without_marking_current() {
        // Anchored above the PC: the PC is not in the walked window, so no row is
        // marked current.
        let memory = Bytes(vec![0x00; 64]);
        let readout = build(Some(&Toy), 40, 8, &memory, None, None, &bus);
        assert!(readout.rows.iter().any(|row| matches!(
            row,
            DisasmRow::Instruction {
                display: AddressDisplay { window: 40, .. },
                ..
            }
        )));
        assert!(!readout.rows.iter().any(|row| matches!(
            row,
            DisasmRow::Instruction {
                is_current: true,
                ..
            }
        )));
    }
}
