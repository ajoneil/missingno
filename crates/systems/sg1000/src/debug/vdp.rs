//! The TMS9918A's register-level section, as the part states it.

use missingno_core::inspect::Section;
use missingno_ti_vdp::inspect::VdpView;

use super::Sg1000InspectState;

pub(crate) fn section(state: &Sg1000InspectState) -> Section {
    missingno_ti_vdp::inspect::section(&VdpView {
        standard: state.standard,
        line: state.line,
        dot: state.dot,
        status: state.vdp_status,
        registers: state.vdp_registers,
        layout: state.vdp_layout.clone(),
    })
}
