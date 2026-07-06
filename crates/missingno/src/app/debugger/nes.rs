//! The NES family's inspection state and debugger panes. One owned state
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
pub struct NesInspectState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    pub scanline: u16,
    pub dot: u16,
    pub ppu_control: u8,
    pub ppu_mask: u8,
    pub ppu_status: u8,
    pub scroll_v: u16,
    pub disassembly: Vec<DisasmRow>,
    pub frame: u64,
}

#[derive(Clone)]
pub struct DisasmRow {
    pub address: u16,
    pub text: String,
    pub current: bool,
}

impl Inspection for NesInspectState {
    fn family_state(&self) -> &dyn std::any::Any {
        self
    }
}

/// The per-frame snapshot for the running view; symbols and code/data logs
/// have no NES backend yet, so it carries empty ones.
pub struct NesSnapshot {
    pub state: NesInspectState,
    symbols: SymbolTable,
    cdl: CdlWindow,
}

impl NesSnapshot {
    pub fn new(state: NesInspectState) -> Self {
        NesSnapshot {
            state,
            symbols: SymbolTable::default(),
            cdl: CdlWindow::default(),
        }
    }
}

impl Inspection for NesSnapshot {
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
}

impl InspectSnapshot for NesSnapshot {
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

fn flag_string(p: u8) -> String {
    "nv-bdizc"
        .chars()
        .enumerate()
        .map(|(i, name)| {
            if p & (0x80 >> i) != 0 {
                name.to_ascii_uppercase()
            } else {
                name
            }
        })
        .collect()
}

pub struct CpuPane;

impl Pane for CpuPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::NesCpu
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(PaneContext::family_state::<NesInspectState>) else {
            return panes::running_placeholder("2A03");
        };
        let mut rows = column![
            mono(format!(
                "pc {:04x}  s {:02x}  p {}",
                state.pc,
                state.s,
                flag_string(state.p)
            )),
            mono(format!(
                "a {:02x}  x {:02x}  y {:02x}",
                state.a, state.x, state.y
            )),
            mono(String::new()),
        ]
        .spacing(s());
        for row in &state.disassembly {
            let marker = if row.current { ">" } else { " " };
            rows = rows.push(mono(format!("{marker} {:04x}  {}", row.address, row.text)));
        }
        panes::pane(panes::title_bar("2A03"), rows.into())
    }
}

pub struct PpuPane;

impl Pane for PpuPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::NesPpu
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(PaneContext::family_state::<NesInspectState>) else {
            return panes::running_placeholder("2C02");
        };
        let rows = column![
            mono(format!(
                "scanline {:3}  dot {:3}",
                state.scanline, state.dot
            )),
            mono(format!(
                "ctrl {:02x}  mask {:02x}  status {:02x}",
                state.ppu_control, state.ppu_mask, state.ppu_status
            )),
            mono(format!("v {:04x}", state.scroll_v)),
        ]
        .spacing(s());
        panes::pane(panes::title_bar("2C02"), rows.into())
    }
}

fn mono<'a>(content: String) -> iced::widget::Text<'a> {
    text(content).font(fonts::monospace())
}
