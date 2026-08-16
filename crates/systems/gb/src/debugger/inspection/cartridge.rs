use missingno_core::inspect;

use crate::cartridge::CartridgeView;

/// The Cartridge section: the mapper, its current bank/enable state, and — on an
/// MBC3 with a clock — the RTC registers. Shared by the DMG and CGB sidebars,
/// and by the live console (paused) and running snapshot, so all agree.
pub fn cartridge_section(cart: &CartridgeView) -> inspect::Section {
    use inspect::{Row, SectionBlock};

    let rom_bank = cart
        .rom_bank
        .map_or_else(|| "—".to_owned(), |bank| bank.to_string());
    let summary = format!("{} · rom {}", cart.mapper, rom_bank);

    let mut rows = vec![
        Row::value("mapper", cart.mapper).help("cartridge memory bank controller"),
        Row::value("rom bank", rom_bank).help("16 KB ROM bank mapped at $4000"),
    ];
    if let Some(bank) = cart.ram_bank {
        rows.push(Row::value("ram bank", bank.to_string()).help("cart-RAM bank mapped at $A000"));
    }
    if let Some(enabled) = cart.ram_enabled {
        rows.push(
            Row::flag("ram enabled", enabled).help("cart-RAM/RTC access latch ($0000-$1FFF)"),
        );
    }
    if let Some(mode1) = cart.mode1 {
        let mode = if mode1 { "1 (advanced)" } else { "0 (simple)" };
        rows.push(Row::value("mode", mode).help("MBC1 banking mode ($6000-$7FFF)"));
    }

    let mut blocks = vec![SectionBlock::Rows(rows)];
    if let Some(rtc) = &cart.rtc {
        blocks.push(SectionBlock::Rule);
        blocks.push(SectionBlock::Rows(vec![
            Row::value("sec", rtc.seconds.to_string()).help("RTC seconds ($08)"),
            Row::value("min", rtc.minutes.to_string()).help("RTC minutes ($09)"),
            Row::value("hour", rtc.hours.to_string()).help("RTC hours ($0A)"),
            Row::value("day", rtc.day.to_string()).help("RTC day counter ($0B, $0C bit 0)"),
            Row::flag("halted", rtc.halted).help("RTC halt ($0C bit 6)"),
            Row::flag("latch armed", rtc.latch_ready).help("$6000 latch awaiting its 01 write"),
            Row::flag("day carry", rtc.day_carry).help("sticky day-counter overflow ($0C bit 7)"),
        ]));
    }

    inspect::Section {
        name: "Cartridge",
        summary,
        active: None,
        detail: None,
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Console;
    use crate::cartridge::Cartridge;
    use crate::debugger::inspection::tests::row_labels;

    #[test]
    fn cartridge_section_shows_mbc3_rtc_rows() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x0f; // MBC3 + TIMER + BATTERY — carries an RTC
        let console = Console::<crate::Dmg>::new(Cartridge::new(rom, None), None);
        let section = cartridge_section(&console.cartridge().inspect());
        assert_eq!(section.name, "Cartridge");
        assert!(section.summary.starts_with("MBC3"), "{}", section.summary);
        let labels = row_labels(&section);
        for expected in ["mapper", "rom bank", "sec", "min", "hour", "day", "halted"] {
            assert!(
                labels.iter().any(|l| l == expected),
                "missing row {expected}"
            );
        }

        // A plain no-clock cart shows the section but no RTC rows.
        let plain = Console::<crate::Dmg>::new(Cartridge::new(vec![0u8; 0x8000], None), None);
        let plain_labels = row_labels(&cartridge_section(&plain.cartridge().inspect()));
        assert!(plain_labels.iter().all(|l| l != "sec"));
    }
}
