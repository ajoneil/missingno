//! The VCS family's inspection state and debugger panes. One owned state
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
pub struct VcsInspectState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    pub beam: u16,
    pub scanline: usize,
    pub timer: u8,
    pub timer_underflowed: bool,
    pub swcha: u8,
    pub swchb: u8,
    pub collisions: [u8; 8],
    pub disassembly: Vec<DisasmRow>,
    pub frame: u64,
}

#[derive(Clone)]
pub struct DisasmRow {
    pub address: u16,
    pub text: String,
    pub current: bool,
}

impl Inspection for VcsInspectState {
    fn as_vcs(&self) -> Option<&VcsInspectState> {
        Some(self)
    }
}

/// The per-frame snapshot for the running view; symbols and code/data logs
/// have no VCS backend yet, so it carries empty ones.
pub struct VcsSnapshot {
    pub state: VcsInspectState,
    symbols: SymbolTable,
    cdl: CdlWindow,
}

impl VcsSnapshot {
    pub fn new(state: VcsInspectState) -> Self {
        VcsSnapshot {
            state,
            symbols: SymbolTable::default(),
            cdl: CdlWindow::default(),
        }
    }
}

impl Inspection for VcsSnapshot {
    fn as_vcs(&self) -> Option<&VcsInspectState> {
        Some(&self.state)
    }
}

impl InspectSnapshot for VcsSnapshot {
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
        DebuggerPane::VcsCpu
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(|ctx| ctx.vcs) else {
            return panes::running_placeholder("6507");
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
        panes::pane(panes::title_bar("6507"), rows.into())
    }
}

pub struct TiaPane;

impl Pane for TiaPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::VcsTia
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(|ctx| ctx.vcs) else {
            return panes::running_placeholder("TIA");
        };
        let collision_names = ["m0p", "m1p", "p0fb", "p1fb", "m0fb", "m1fb", "blpf", "ppmm"];
        let mut rows = column![
            mono(format!("beam {:3}  line {:3}", state.beam, state.scanline)),
            mono(format!(
                "timer {:02x}{}",
                state.timer,
                if state.timer_underflowed {
                    " (expired)"
                } else {
                    ""
                }
            )),
            mono(format!(
                "swcha {:02x}  swchb {:02x}",
                state.swcha, state.swchb
            )),
            mono(String::new()),
        ]
        .spacing(s());
        for (name, value) in collision_names.iter().zip(state.collisions.iter()) {
            rows = rows.push(mono(format!("cx{name:<5} {value:02x}")));
        }
        panes::pane(panes::title_bar("TIA"), rows.into())
    }
}

fn mono<'a>(content: String) -> iced::widget::Text<'a> {
    text(content).font(fonts::monospace())
}
