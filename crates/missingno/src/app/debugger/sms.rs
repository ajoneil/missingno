//! The SMS family's debugger panes, rendering the crate-owned inspection
//! state the running and paused views share.

use iced::widget::{column, pane_grid, text};
use missingno_sms::debug::SmsInspectState;

use crate::app;
use crate::app::debugger::panes::{self, DebuggerPane, Pane, PaneContext};
use crate::app::ui::{fonts, sizes::s};

pub struct CpuPane;

impl Pane for CpuPane {
    fn kind(&self) -> DebuggerPane {
        DebuggerPane::SmsCpu
    }

    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message> {
        let Some(state) = ctx.and_then(PaneContext::family_state::<SmsInspectState>) else {
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
        let Some(state) = ctx.and_then(PaneContext::family_state::<SmsInspectState>) else {
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
