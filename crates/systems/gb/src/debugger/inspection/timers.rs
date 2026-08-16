use missingno_core::inspect;

/// The timer registers plus the internal divider counter, captured so the live
/// console (paused) and the running snapshot serve the same section.
#[derive(Clone, Copy)]
pub struct TimersView {
    /// DIV ($FF04) — the divider's upper byte.
    pub div: u8,
    /// TIMA ($FF05) — the counter.
    pub tima: u8,
    /// TMA ($FF06) — the reload modulo.
    pub tma: u8,
    /// TAC ($FF07) — the control byte.
    pub tac: u8,
    /// The full 16-bit internal divider counter DIV reads its byte from.
    pub internal_counter: u16,
}

impl TimersView {
    pub fn capture(timers: &crate::timers::Timers) -> Self {
        use crate::timers::Register;
        Self {
            div: timers.read_register(Register::Divider),
            tima: timers.read_register(Register::Counter),
            tma: timers.read_register(Register::Modulo),
            tac: timers.read_register(Register::Control),
            internal_counter: timers.internal_counter(),
        }
    }
}

/// The Timers section: the DIV/TIMA/TMA/TAC registers, the TAC enable pip and
/// decoded increment frequency, and the internal 16-bit divider counter DIV is
/// a window onto. Shared by DMG and CGB (the same timer silicon).
pub fn timers_section(timers: &TimersView) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let enabled = timers.tac & 0b100 != 0;
    let frequency = match timers.tac & 0b11 {
        0b00 => 4096,
        0b01 => 262144,
        0b10 => 65536,
        _ => 16384,
    };

    inspect::Section {
        name: "Timers",
        summary: format!("div {:02X} · tima {:02X}", timers.div, timers.tima),
        active: Some(enabled),
        detail: None,
        blocks: vec![SectionBlock::Rows(vec![
            Row::value("div", format!("{:02X}", timers.div)).help("divider register (FF04)"),
            Row::value("tima", format!("{:02X}", timers.tima)).help("timer counter (FF05)"),
            Row::value("tma", format!("{:02X}", timers.tma)).help("timer modulo — reload (FF06)"),
            Row::value("tac", format!("{:02X}", timers.tac)).help("timer control (FF07)"),
            Row::flag("enabled", enabled).help("timer enable (TAC bit 2)"),
            Row::value("freq", format!("{frequency} Hz"))
                .help("TIMA increment frequency (TAC bits 0-1)"),
            Row::value("counter", format!("{:04X}", timers.internal_counter))
                .help("internal 16-bit divider counter"),
        ])],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspection::tests::{row_labels, stepped_dmg};

    #[test]
    fn timers_section_carries_registers_and_divider_width() {
        let debugger = stepped_dmg();
        let timers = TimersView::capture(debugger.game_boy().timers());
        let section = timers_section(&timers);
        assert_eq!(section.name, "Timers");
        let labels = row_labels(&section);
        for expected in ["div", "tima", "tma", "tac", "enabled", "freq", "counter"] {
            assert!(
                labels.iter().any(|l| l == expected),
                "missing row {expected}"
            );
        }
        // The internal divider counter is the full 16-bit value — four hex digits.
        let counter = section
            .blocks
            .iter()
            .find_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows
                    .iter()
                    .find(|row| row.label == "counter")
                    .map(|row| row.value.clone()),
                _ => None,
            })
            .expect("a counter row");
        assert_eq!(counter.len(), 4, "divider counter is 16-bit hex: {counter}");
    }
}
