//! Watch conditions: what the debugger can be asked to stop on, and how a
//! condition maps to and from the seam's flat term vocabulary.
//!
//! A condition is evaluated at an instruction boundary — the same point a plain
//! PC breakpoint is checked — so a watch never fires mid-instruction.

use missingno_core::inspect;

use crate::console::Vcs;

/// The watchable key the disassembly gutter composes into a `{pc, cart-bank}`
/// watch on a banked-window row. Shared with `present_address` so a row's bank
/// watch and the exposed key cannot drift.
pub(crate) const CART_BANK_KEY: &str = "cart-bank";

/// A watch condition, evaluated at each instruction boundary — the same point a
/// plain PC breakpoint is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WatchCondition {
    /// The CPU reaches this address (compared on the 13 decoded lines) — the
    /// instruction-boundary point a plain breakpoint fires on.
    Pc(u16),
    /// The board pages this 4 KB bank into the cart window. Reads nothing on an
    /// unbanked board, so it never matches there.
    CartBank(u16),
    /// A conjunction: every condition must hold.
    All(Vec<WatchCondition>),
}

impl WatchCondition {
    /// Whether the condition holds for the console as it stands.
    pub(super) fn matches(&self, vcs: &Vcs) -> bool {
        match self {
            // The 6507 decodes 13 address lines: match on them, as breakpoints do.
            WatchCondition::Pc(target) => vcs.cpu.pc & 0x1FFF == target & 0x1FFF,
            WatchCondition::CartBank(target) => {
                vcs.cartridge().selected_bank() == Some(*target as usize)
            }
            WatchCondition::All(conditions) => conditions.iter().all(|c| c.matches(vcs)),
        }
    }
}

static WATCHABLES: &[inspect::Watchable] = &[
    // A full 16-bit address; the condition compares it on the 13 decoded lines.
    inspect::Watchable {
        key: "pc",
        label: "PC",
        param: inspect::WatchParam::Value { bits: 16 },
    },
    inspect::Watchable {
        key: CART_BANK_KEY,
        label: "cart bank",
        param: inspect::WatchParam::Value { bits: 16 },
    },
];

/// The watchables the debugger exposes.
pub fn watchables() -> &'static [inspect::Watchable] {
    WATCHABLES
}

fn condition_from_term(term: &inspect::WatchTerm) -> Option<WatchCondition> {
    let value = term.value?;
    match term.key.as_str() {
        "pc" => Some(WatchCondition::Pc(value as u16)),
        key if key == CART_BANK_KEY => Some(WatchCondition::CartBank(value as u16)),
        _ => None,
    }
}

fn term_from_condition(condition: &WatchCondition) -> inspect::WatchTerm {
    let (key, value) = match condition {
        WatchCondition::Pc(address) => ("pc", *address as u32),
        WatchCondition::CartBank(bank) => (CART_BANK_KEY, *bank as u32),
        WatchCondition::All(_) => unreachable!("compounds are flattened before term conversion"),
    };
    inspect::WatchTerm {
        key: key.to_string(),
        address: None,
        value: Some(value),
    }
}

fn flatten_terms(condition: &WatchCondition, out: &mut Vec<inspect::WatchTerm>) {
    match condition {
        WatchCondition::All(conditions) => {
            for condition in conditions {
                flatten_terms(condition, out);
            }
        }
        leaf => out.push(term_from_condition(leaf)),
    }
}

pub(super) fn watch_from_condition(condition: &WatchCondition) -> inspect::Watch {
    let mut terms = Vec::new();
    flatten_terms(condition, &mut terms);
    inspect::Watch { terms }
}

/// Whether every term of `watch` names something this side can evaluate.
pub fn supports_watch(watch: &inspect::Watch) -> bool {
    watch_to_condition(watch).is_some()
}

pub(super) fn watch_to_condition(watch: &inspect::Watch) -> Option<WatchCondition> {
    let mut conditions = Vec::with_capacity(watch.terms.len());
    for term in &watch.terms {
        conditions.push(condition_from_term(term)?);
    }
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(WatchCondition::All(conditions)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::CartType;
    use crate::debugger::test_support::{debugger, reset_to_f000};
    use crate::debugger::{Debugger, Stop, Stops};
    use missingno_core::machine::StopSet;

    /// The seam's stores holding only these watches.
    fn stops(watches: &[inspect::Watch]) -> Stops {
        Stops::new(&StopSet {
            pc: Default::default(),
            watches: watches.to_vec(),
        })
    }

    /// Step until a stop fires, bounded the way the seam's run hook bounds it.
    fn run_to_stop(debugger: &mut Debugger, stops: &Stops) -> Option<Stop> {
        (0..200_000).find_map(|_| debugger.step(stops))
    }

    fn pc_watch(address: u16) -> inspect::Watch {
        inspect::Watch::single("pc", None, Some(address as u32))
    }

    fn pc_bank_watch(address: u16, bank: u16) -> inspect::Watch {
        inspect::Watch {
            terms: vec![
                inspect::WatchTerm {
                    key: "pc".into(),
                    address: None,
                    value: Some(address as u32),
                },
                inspect::WatchTerm {
                    key: CART_BANK_KEY.into(),
                    address: None,
                    value: Some(bank as u32),
                },
            ],
        }
    }

    #[test]
    fn pc_watch_stops_at_the_address() {
        // NOP at $F000, then JMP $F001 self-loop; a pc watch at $F001 stops there.
        let mut rom = vec![0u8; 0x1000];
        rom[0x000..0x004].copy_from_slice(&[0xEA, 0x4C, 0x01, 0xF0]);
        reset_to_f000(&mut rom);
        let mut debugger = debugger(&rom, CartType::Plain4K);
        let stop = run_to_stop(&mut debugger, &stops(&[pc_watch(0xF001)]));
        assert!(matches!(stop, Some(Stop::Watch(_))));
        assert_eq!(debugger.pc() & 0x1FFF, 0xF001 & 0x1FFF);
    }

    /// An F8 board (two 4 KB banks) whose identical banks run three NOPs, switch
    /// to bank 1 by touching the `$FFF9` hotspot, then self-loop at `$F006`.
    fn f8_bank_switch_rom() -> Vec<u8> {
        let mut bank = vec![0u8; 0x1000];
        bank[0x000..0x009].copy_from_slice(&[
            0xEA, 0xEA, 0xEA, // three NOPs → $F000..$F003
            0xAD, 0xF9, 0xFF, // LDA $FFF9 — hotspot, selects bank 1
            0x4C, 0x06, 0xF0, // JMP $F006 self-loop
        ]);
        reset_to_f000(&mut bank);
        [bank.clone(), bank].concat()
    }

    #[test]
    fn cart_bank_watch_gates_on_the_selected_bank() {
        // Reached before the hotspot: $F002 runs on the wake bank (0).
        let mut on_bank0 = debugger(&f8_bank_switch_rom(), CartType::Atari8K);
        let stop = run_to_stop(&mut on_bank0, &stops(&[pc_bank_watch(0xF002, 0)]));
        assert!(matches!(stop, Some(Stop::Watch(_))));
        assert_eq!(on_bank0.console().cartridge().selected_bank(), Some(0));

        // At the loop the board has switched to bank 1. A `{pc, cart-bank:0}`
        // watch is held first but must NOT match there; the `cart-bank:1` watch
        // does — proving the bank term gates the compound.
        let mut on_bank1 = debugger(&f8_bank_switch_rom(), CartType::Atari8K);
        let held = stops(&[pc_bank_watch(0xF006, 0), pc_bank_watch(0xF006, 1)]);
        let stop = run_to_stop(&mut on_bank1, &held);
        assert_eq!(on_bank1.console().cartridge().selected_bank(), Some(1));
        let Some(Stop::Watch(hit)) = stop else {
            panic!("a watch fired");
        };
        let bank_term = hit
            .terms
            .iter()
            .find(|t| t.key == CART_BANK_KEY)
            .expect("carries the bank term");
        assert_eq!(bank_term.value, Some(1));
    }

    #[test]
    fn watchables_expose_pc_and_cart_bank() {
        let keys: Vec<&str> = watchables().iter().map(|w| w.key).collect();
        assert!(keys.contains(&"pc"));
        assert!(keys.contains(&CART_BANK_KEY));
    }
}
