//! The SMS family's inspection state and debugger panes. One owned state
//! struct serves both the paused view (refreshed after every step) and the
//! per-frame snapshot the running view renders from.

use iced::widget::{column, pane_grid, text};

use crate::app;
use crate::app::debugger::inspect::{InspectSnapshot, Inspection};
use crate::app::debugger::panes::{self, DebuggerPane, Pane, PaneContext};
use crate::app::ui::{fonts, sizes::s};
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::symbols::SymbolTable;

#[derive(Clone, Default)]
pub struct SmsInspectState {
    pub a: u8,
    pub f: u8,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub line: u16,
    pub dot: u16,
    pub vdp_status: u8,
    pub vdp_registers: [u8; 11],
    pub banks: [u8; 3],
    /// Raw bytes at the program counter, hex-dumped until a Z80
    /// disassembler lands.
    pub code_window: Vec<(u16, [u8; 4])>,
    pub frame: u64,
}

impl Inspection for SmsInspectState {
    fn as_sms(&self) -> Option<&SmsInspectState> {
        Some(self)
    }
}

/// The per-frame snapshot for the running view; symbols and code/data logs
/// have no SMS backend yet, so it carries empty ones.
pub struct SmsSnapshot {
    pub state: SmsInspectState,
    symbols: SymbolTable,
    cdl: CdlWindow,
}

impl SmsSnapshot {
    pub fn new(state: SmsInspectState) -> Self {
        SmsSnapshot {
            state,
            symbols: SymbolTable::default(),
            cdl: CdlWindow::default(),
        }
    }
}

impl Inspection for SmsSnapshot {
    fn as_sms(&self) -> Option<&SmsInspectState> {
        Some(&self.state)
    }
}

impl InspectSnapshot for SmsSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }
    fn cdl(&self) -> &CdlWindow {
        &self.cdl
    }
}

pub struct CpuPane;

impl Pane for CpuPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::SmsCpu
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(|ctx| ctx.sms) else {
            return panes::running_placeholder("Z80");
        };
        let mut rows = column![
            mono(format!(
                "pc {:04x}  sp {:04x}  af {:02x}{:02x}",
                state.pc, state.sp, state.a, state.f
            )),
            mono(format!(
                "bc {:04x}  de {:04x}  hl {:04x}",
                state.bc, state.de, state.hl
            )),
            mono(format!("ix {:04x}  iy {:04x}", state.ix, state.iy)),
            mono(format!(
                "banks {:02x} {:02x} {:02x}",
                state.banks[0], state.banks[1], state.banks[2]
            )),
            mono(String::new()),
        ]
        .spacing(s());
        for (address, bytes) in &state.code_window {
            rows = rows.push(mono(format!(
                "{address:04x}  {:02x} {:02x} {:02x} {:02x}",
                bytes[0], bytes[1], bytes[2], bytes[3]
            )));
        }
        panes::pane(panes::title_bar("Z80"), rows.into())
    }
}

pub struct VdpPane;

impl Pane for VdpPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::SmsVdp
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(|ctx| ctx.sms) else {
            return panes::running_placeholder("VDP");
        };
        let mut rows = column![
            mono(format!("line {:3}  dot {:3}", state.line, state.dot)),
            mono(format!("status {:02x}", state.vdp_status)),
            mono(String::new()),
        ]
        .spacing(s());
        for (index, value) in state.vdp_registers.iter().enumerate() {
            rows = rows.push(mono(format!("r{index:<2} {value:02x}")));
        }
        panes::pane(panes::title_bar("VDP"), rows.into())
    }
}

fn mono<'a>(content: String) -> iced::widget::Text<'a> {
    text(content).font(fonts::monospace())
}
