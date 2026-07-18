use std::collections::BTreeSet;

use iced::{
    Length,
    widget::{Column, pane_grid, text},
};

use crate::app::{
    self,
    debugger::{
        disasm_rows,
        panes::{
            self, DebuggerPane, PaneContext, pane, running_placeholder, title_bar,
            title_bar_with_detail,
        },
    },
    ui::{fonts, palette},
};
use missingno_core::cdl::CdlWindow;
use missingno_core::symbols::SymbolTable;
use missingno_gb::cpu::instructions::Instruction;
use missingno_gb::debugger::instructions::{
    InstructionsIterator, ReadInstructionMemory, Row, addresses_before, rows_from,
};
use missingno_gb::isa::Sm83;

/// Number of instructions to show before the current PC.
const CONTEXT_BEFORE: usize = 4;
/// Number of instructions to show after (and including) the current PC.
const CONTEXT_AFTER: usize = 80;

pub struct InstructionsPane;

impl InstructionsPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(
        &self,
        memory: &dyn ReadInstructionMemory,
        pc: u16,
        breakpoints: &BTreeSet<u32>,
        symbols: &SymbolTable,
        rom_bank: Option<u16>,
        cdl: &CdlWindow,
    ) -> pane_grid::Content<'_, app::Message> {
        // A label resolves against the bank mapped in the switchable region.
        let bank_at = |address: u16| {
            if (0x4000..0x8000).contains(&address) {
                rom_bank
            } else {
                None
            }
        };
        let mut instructions = Vec::new();
        let push_label = |rows: &mut Vec<_>, address: u16| {
            if let Some(label) = symbols.label_at(address, rom_bank) {
                rows.push(disasm_rows::label_row(label));
            }
        };
        let push_instruction =
            |rows: &mut Vec<_>, address: u16, decoded: Instruction, is_current: bool| {
                let tokens = disasm_rows::tokenize(&Sm83, &decoded.to_string());
                rows.push(disasm_rows::instruction_row(
                    address as u32,
                    bank_at(address),
                    &tokens,
                    is_current,
                    breakpoints.contains(&(address as u32)),
                ));
            };

        // Instructions before PC: exact where the code/data log has seen
        // execution, falling back to the heuristic sweep where it hasn't.
        let before = addresses_before(pc, CONTEXT_BEFORE, memory, Some(cdl));
        for &addr in &before {
            let mut iter = InstructionsIterator::new(addr, memory);
            if let Some(decoded) = Instruction::decode(&mut iter) {
                push_label(&mut instructions, addr);
                push_instruction(&mut instructions, addr, decoded, false);
            }
        }

        // Instructions from PC onwards; bytes the code/data log knows were
        // never executed render as data instead of decoding through them.
        for row in rows_from(pc, CONTEXT_AFTER, memory, Some(cdl)) {
            match row {
                Row::Data(address) => {
                    let address = address as u16;
                    push_label(&mut instructions, address);
                    instructions.push(disasm_rows::data_row(
                        address as u32,
                        bank_at(address),
                        memory.read(address),
                    ));
                }
                Row::Instruction(address) => {
                    let address = address as u16;
                    let mut iter = InstructionsIterator::new(address, memory);
                    if let Some(decoded) = Instruction::decode(&mut iter) {
                        push_label(&mut instructions, address);
                        push_instruction(&mut instructions, address, decoded, address == pc);
                    }
                }
            }
        }

        let header = if breakpoints.is_empty() {
            title_bar("Instructions")
        } else {
            let detail = format!("{} bp", breakpoints.len(),);
            title_bar_with_detail(
                "Instructions",
                text(detail)
                    .font(fonts::monospace())
                    .size(11.0)
                    .color(palette::MUTED),
            )
        };

        pane(
            header,
            iced::widget::scrollable(Column::from_vec(instructions).width(Length::Fill))
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

impl panes::Pane for InstructionsPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::Instructions
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        match ctx.and_then(|ctx| ctx.gb.map(|gb| (ctx, gb))) {
            Some((ctx, gb)) => self.content(
                gb.source.instruction_memory(),
                gb.source.cpu().ir_address(),
                ctx.breakpoints,
                gb.symbols,
                gb.source.switchable_rom_bank(),
                gb.cdl,
            ),
            None => running_placeholder("Instructions"),
        }
    }
}
