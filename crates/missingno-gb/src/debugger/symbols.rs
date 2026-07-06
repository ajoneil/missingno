//! Debug symbol tables in the no$gmb/RGBDS `.sym` format.
//!
//! One label per line as `bank:address name` (hex bank and CPU address),
//! `;` comments, blank lines ignored. WLA-DX section headers like `[labels]`
//! are tolerated; lines that don't fit the grammar are skipped.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub bank: u16,
    /// CPU address (0x0000–0xFFFF), not a flat ROM offset.
    pub address: u16,
    pub name: String,
}

/// Labels indexed by CPU address, loaded from a ROM's `.sym` sidecar.
#[derive(Default, Clone)]
pub struct SymbolTable {
    by_address: HashMap<u16, Vec<Symbol>>,
    len: usize,
}

impl SymbolTable {
    pub fn parse(text: &str) -> Self {
        let mut table = Self::default();
        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let Some((location, name)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            let Some((bank, address)) = location.split_once(':') else {
                continue;
            };
            let (Ok(bank), Ok(address)) = (
                u16::from_str_radix(bank, 16),
                u16::from_str_radix(address, 16),
            ) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                continue;
            }
            table.by_address.entry(address).or_default().push(Symbol {
                bank,
                address,
                name: name.to_string(),
            });
            table.len += 1;
        }
        table
    }

    /// The `.sym` sidecar next to a ROM (`game.gb` → `game.sym`); empty when
    /// there isn't one.
    pub fn for_rom(rom_path: &Path) -> Self {
        fs::read_to_string(rom_path.with_extension("sym"))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// The label at a CPU address. In the switchable-ROM region the label is
    /// only resolved when the mapped bank is known and matches, or when a
    /// single bank defines the address — an unbanked or single-candidate
    /// label can't mislead, an ambiguous one could.
    pub fn label_at(&self, address: u16, mapped_rom_bank: Option<u16>) -> Option<&str> {
        let candidates = self.by_address.get(&address)?;
        let switchable_rom = (0x4000..0x8000).contains(&address);
        match (switchable_rom, mapped_rom_bank) {
            (true, Some(bank)) => candidates.iter().find(|symbol| symbol.bank == bank),
            _ => match candidates.as_slice() {
                [only] => Some(only),
                _ => None,
            },
        }
        .map(|symbol| symbol.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labels_comments_and_sections() {
        let table = SymbolTable::parse(
            "; a comment\n\
             [labels]\n\
             00:0150 Main\n\
             00:0040 VBlankInterrupt ; trailing comment\n\
             03:47F2 Read_Joypad_State\n\
             0001:5678 WlaSixteenBitBank\n\
             00:C0A0 wPlayerX\n\
             deadbeef NotALabel\n\
             00:zzzz AlsoNot\n",
        );
        assert_eq!(table.len(), 5);
        assert_eq!(table.label_at(0x0150, None), Some("Main"));
        assert_eq!(table.label_at(0x0040, None), Some("VBlankInterrupt"));
        assert_eq!(table.label_at(0xC0A0, None), Some("wPlayerX"));
        assert_eq!(table.label_at(0x5678, None), Some("WlaSixteenBitBank"));
        assert_eq!(table.label_at(0x0100, None), None);
    }

    #[test]
    fn banked_rom_labels_need_an_unambiguous_or_matching_bank() {
        let table = SymbolTable::parse(
            "01:4000 BankOneEntry\n\
             02:4000 BankTwoEntry\n\
             03:47F2 OnlyBankThree\n",
        );
        assert_eq!(table.label_at(0x4000, None), None);
        assert_eq!(table.label_at(0x4000, Some(2)), Some("BankTwoEntry"));
        assert_eq!(table.label_at(0x4000, Some(7)), None);
        assert_eq!(table.label_at(0x47F2, None), Some("OnlyBankThree"));
        assert_eq!(table.label_at(0x47F2, Some(3)), Some("OnlyBankThree"));
    }
}
