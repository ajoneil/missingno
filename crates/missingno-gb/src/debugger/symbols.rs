//! Debug symbol tables in the no$gmb/RGBDS `.sym` format.
//!
//! One label per line as `bank:address name` (hex bank and CPU address),
//! `;` comments, blank lines ignored. WLA-DX section headers like `[labels]`
//! are tolerated; lines that don't fit the grammar are skipped.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Section header separating user-created labels from the tool-generated
/// body of a `.sym` file, so edits survive without rewriting the original.
const USER_SECTION: &str = "; missingno user labels";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub bank: u16,
    /// CPU address (0x0000–0xFFFF), not a flat ROM offset.
    pub address: u16,
    pub name: String,
}

/// Labels indexed by CPU address, loaded from a ROM's `.sym` sidecar.
/// User-created labels are tracked separately so they stay editable and
/// write back without disturbing a tool-generated file.
#[derive(Default, Clone)]
pub struct SymbolTable {
    by_address: HashMap<u16, Vec<Symbol>>,
    len: usize,
    user: Vec<Symbol>,
    dirty: bool,
}

impl SymbolTable {
    pub fn parse(text: &str) -> Self {
        let mut table = Self::default();
        let mut in_user_section = false;
        for line in text.lines() {
            if line.trim() == USER_SECTION {
                in_user_section = true;
                continue;
            }
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
            let symbol = Symbol {
                bank,
                address,
                name: name.to_string(),
            };
            if in_user_section {
                table.user.push(symbol.clone());
            }
            table.by_address.entry(address).or_default().push(symbol);
            table.len += 1;
        }
        table
    }

    /// Add a user label, replacing any existing user label at the same spot.
    pub fn add_user(&mut self, symbol: Symbol) {
        if symbol.name.is_empty() || symbol.name.contains(char::is_whitespace) {
            return;
        }
        self.remove_user_at(symbol.bank, symbol.address);
        self.by_address
            .entry(symbol.address)
            .or_default()
            .push(symbol.clone());
        self.user.push(symbol);
        self.len += 1;
        self.dirty = true;
    }

    /// Remove a user-created label. Labels from the generated body of the
    /// file aren't removable — we never rewrite that part.
    pub fn remove_user(&mut self, symbol: &Symbol) {
        if self.user.iter().any(|s| s == symbol) {
            self.remove_user_at(symbol.bank, symbol.address);
        }
    }

    fn remove_user_at(&mut self, bank: u16, address: u16) {
        let matches = |s: &Symbol| s.bank == bank && s.address == address;
        let user_names: Vec<String> = self
            .user
            .iter()
            .filter(|s| matches(s))
            .map(|s| s.name.clone())
            .collect();
        if user_names.is_empty() {
            return;
        }
        self.user.retain(|s| !matches(s));
        if let Some(symbols) = self.by_address.get_mut(&address) {
            symbols.retain(|s| !(matches(s) && user_names.contains(&s.name)));
            self.len -= user_names.len();
        }
        self.dirty = true;
    }

    pub fn user_symbols(&self) -> &[Symbol] {
        &self.user
    }

    /// Write user labels back to the sidecar: everything above our section
    /// header is preserved verbatim, the section is rewritten below it.
    pub fn save(&self, path: &Path) {
        if !self.dirty {
            return;
        }
        let existing = fs::read_to_string(path).unwrap_or_default();
        let output = self.render(&existing);
        if output.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            let _ = fs::write(path, output);
        }
    }

    fn render(&self, existing: &str) -> String {
        let mut output = match existing.find(USER_SECTION) {
            Some(marker) => existing[..marker].to_string(),
            None if existing.is_empty() => String::new(),
            None => {
                let mut kept = existing.to_string();
                if !kept.ends_with('\n') {
                    kept.push('\n');
                }
                kept
            }
        };
        if !self.user.is_empty() {
            output.push_str(USER_SECTION);
            output.push('\n');
            for symbol in &self.user {
                output.push_str(&format!(
                    "{:02X}:{:04X} {}\n",
                    symbol.bank, symbol.address, symbol.name
                ));
            }
        }
        output
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
    fn user_labels_round_trip_through_the_sidecar_section() {
        let generated = "00:0150 Main\n03:47F2 Read_Joypad_State\n";
        let mut table = SymbolTable::parse(generated);
        table.add_user(Symbol {
            bank: 0,
            address: 0xc0a0,
            name: "myCounter".to_string(),
        });
        table.add_user(Symbol {
            bank: 2,
            address: 0x4321,
            name: "MyRoutine".to_string(),
        });
        assert_eq!(table.label_at(0xc0a0, None), Some("myCounter"));

        let written = table.render(generated);
        assert!(written.starts_with(generated));
        assert!(written.contains("; missingno user labels\n00:C0A0 myCounter"));

        // A reload sees the user labels as user labels again.
        let mut reloaded = SymbolTable::parse(&written);
        assert_eq!(reloaded.user_symbols().len(), 2);
        assert_eq!(reloaded.label_at(0x4321, Some(2)), Some("MyRoutine"));

        // Renaming replaces; removing only touches the user section.
        reloaded.add_user(Symbol {
            bank: 0,
            address: 0xc0a0,
            name: "renamed".to_string(),
        });
        assert_eq!(reloaded.label_at(0xc0a0, None), Some("renamed"));
        let victim = reloaded.user_symbols()[0].clone();
        reloaded.remove_user(&victim);
        let rewritten = reloaded.render(&written);
        assert!(rewritten.starts_with(generated));
        assert!(!rewritten.contains(&victim.name));
        assert_eq!(reloaded.label_at(0x0150, None), Some("Main"));
    }

    #[test]
    fn generated_labels_are_not_removable() {
        let mut table = SymbolTable::parse("00:0150 Main\n");
        table.remove_user(&Symbol {
            bank: 0,
            address: 0x0150,
            name: "Main".to_string(),
        });
        assert_eq!(table.label_at(0x0150, None), Some("Main"));
        assert_eq!(table.len(), 1);
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
