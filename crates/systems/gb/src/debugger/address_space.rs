use crate::Model;
use missingno_core::inspect;

use super::{Debugger, ROM_BANK_KEY, SRAM_BANK_KEY, WRAM_BANK_KEY};

// Synthetic address bases above the real bus, where the debugger exposes
// bank-complete cartridge stores past the CPU's bank-selected windows. Each
// gets its own decade with room for the largest image the family allows (GB ROM
// reaches 8 MB), and the same scheme is mirrored in the VCS debugger.
/// Bank-complete cartridge RAM, all banks linear.
const RAM_BASE: u32 = 0x0100_0000;
/// The full ROM image, all banks in file order.
const ROM_BASE: u32 = 0x0200_0000;
/// Bank-complete work RAM, all banks linear (CGB's eight banks; DMG has none).
const WRAM_BASE: u32 = 0x0300_0000;
/// Bank-complete video RAM, both banks linear (CGB's two banks; DMG has none).
const VRAM_BASE: u32 = 0x0400_0000;

/// Which bank-complete store a synthetic address resolves to.
#[derive(Clone, Copy)]
enum SyntheticStore {
    Rom,
    Sram,
    Wram,
    Vram,
}

impl<M: Model> Debugger<M> {
    /// The cartridge's bank-complete stores in the synthetic space above the
    /// bus, each bounded by its image length: the full ROM image, the cart RAM,
    /// and the banked work-RAM image (CGB). Shared by [`memory_regions`],
    /// [`peek`](Self::peek) and [`present_address`](Self::present_address) so the
    /// published bounds and the routing cannot drift. A store the cart lacks has
    /// length 0 and so contains no address.
    fn synthetic_regions(&self) -> [(SyntheticStore, inspect::MemoryRegion); 4] {
        let cartridge = self.game_boy.cartridge();
        let wram_len = self
            .game_boy
            .model()
            .wram_image()
            .map_or(0, |wram| wram.len() as u32);
        let vram_len = self.game_boy.vram_image_len().unwrap_or(0);
        let region = |name, start, len| inspect::MemoryRegion { name, start, len };
        [
            (
                SyntheticStore::Rom,
                region("rom", ROM_BASE, cartridge.rom_len() as u32),
            ),
            (
                SyntheticStore::Sram,
                region("sram", RAM_BASE, cartridge.ram_len() as u32),
            ),
            (
                SyntheticStore::Wram,
                region("wram-all", WRAM_BASE, wram_len),
            ),
            (
                SyntheticStore::Vram,
                region("vram-all", VRAM_BASE, vram_len),
            ),
        ]
    }

    /// The synthetic store `address` falls in and its linear offset within it,
    /// bounded by the region table — `None` for a bus address or one past every
    /// store.
    fn synthetic_route(&self, address: u32) -> Option<(SyntheticStore, u32)> {
        self.synthetic_regions()
            .into_iter()
            .find(|(_, region)| region.contains(address))
            .map(|(store, region)| (store, address - region.start))
    }

    /// The CPU-visible flat address map, named by role, plus the cartridge's
    /// bank-complete stores in the synthetic space above the bus: the full ROM
    /// image, `sram` when the cart has RAM, and `wram-all` on a console that
    /// banks work RAM (CGB). The bus-window regions (`rom0`/`romx`/`extram`) stay
    /// the CPU's bank-selected, enable-gated view.
    pub fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
        const fn region(name: &'static str, start: u32, len: u32) -> inspect::MemoryRegion {
            inspect::MemoryRegion { name, start, len }
        }
        let mut regions = vec![
            region("rom0", 0x0000, 0x4000),
            region("romx", 0x4000, 0x4000),
            region("vram", 0x8000, 0x2000),
            region("extram", 0xA000, 0x2000),
            region("wram", 0xC000, 0x2000),
            region("oam", 0xFE00, 0xA0),
            region("io", 0xFF00, 0x80),
            region("hram", 0xFF80, 0x7F),
        ];
        // The ROM image is always present; the RAM stores appear only when the
        // cart (or console) carries them.
        for (store, region) in self.synthetic_regions() {
            if matches!(store, SyntheticStore::Rom) || region.len > 0 {
                regions.push(region);
            }
        }
        regions
    }

    /// Side-effect-free read of the CPU address space. Addresses in the
    /// synthetic bank-complete space read the cart's raw ROM or RAM, or the
    /// banked work RAM, linearly — independent of the current bank; below it,
    /// the CPU bus. An address above the bus but past every store reads open bus.
    pub fn peek(&self, address: u32) -> u8 {
        let cartridge = self.game_boy.cartridge();
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => cartridge.peek_rom(offset as usize),
            Some((SyntheticStore::Sram, offset)) => cartridge.peek_ram(offset as usize),
            Some((SyntheticStore::Wram, offset)) => self
                .game_boy
                .model()
                .wram_image()
                .and_then(|wram| wram.get(offset as usize).copied())
                .unwrap_or(0xFF),
            Some((SyntheticStore::Vram, offset)) => self.game_boy.vram_image_byte(offset),
            None if address <= u16::MAX as u32 => self.game_boy.peek(address as u16),
            None => 0xFF,
        }
    }

    /// How `address` presents in the disassembly's address column: a synthetic
    /// bank-complete address as its bank and the CPU window it pages into (ROM
    /// bank 0 at `$0000`, banks ≥1 at `$4000`; SRAM at `$A000`; WRAM bank 0 at
    /// `$C000`, banks ≥1 at `$D000`), a plain bus address as itself. A
    /// breakpoint from a switchable-window row would fire for whichever bank is
    /// paged in, so only fixed-bank windows carry one.
    pub fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        use inspect::AddressDisplay;
        match self.synthetic_route(address) {
            Some((SyntheticStore::Rom, offset)) => {
                let bank = (offset / 0x4000) as u16;
                if bank == 0 {
                    AddressDisplay::fixed(offset, 0)
                } else {
                    AddressDisplay::banked(0x4000 + (offset % 0x4000), bank, ROM_BANK_KEY)
                }
            }
            Some((SyntheticStore::Sram, offset)) => AddressDisplay::banked(
                0xA000 + (offset % 0x2000),
                (offset / 0x2000) as u16,
                SRAM_BANK_KEY,
            ),
            Some((SyntheticStore::Wram, offset)) => {
                let bank = (offset / 0x1000) as u16;
                if bank == 0 {
                    AddressDisplay::fixed(0xC000 + offset, 0)
                } else {
                    AddressDisplay::banked(0xD000 + (offset % 0x1000), bank, WRAM_BANK_KEY)
                }
            }
            // Both VRAM banks page into the same $8000 window (VBK-switched);
            // there is no VBK bank watch, so the bank shows for orientation only.
            Some((SyntheticStore::Vram, offset)) => {
                AddressDisplay::shared_window(0x8000 + (offset % 0x2000), (offset / 0x2000) as u16)
            }
            None => {
                let bank = match address as u16 {
                    0x4000..=0x7FFF => self.game_boy.cartridge().switchable_rom_bank(),
                    _ => None,
                };
                AddressDisplay::bus(address, bank)
            }
        }
    }

    /// The synthetic bank-complete address whose row presents as `bank:window`,
    /// for jump-to-address — the inverse of [`present_address`](Self::present_address)
    /// over the synthetic space. `None` when no region carries that pairing. ROM
    /// bank 0 presents only through the fixed `$0000` window, so a `$4000`-window
    /// pairing with bank 0 has no synthetic row and is rejected.
    pub fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        let cartridge = self.game_boy.cartridge();
        let wram_len = self.game_boy.model().wram_image().map(<[u8]>::len);
        let vram_len = self.game_boy.vram_image_len();
        match window {
            0x0000..=0x3FFF if bank == 0 => {
                (window < cartridge.rom_len() as u32).then_some(ROM_BASE + window)
            }
            0x4000..=0x7FFF if bank != 0 => {
                let linear = bank as u32 * 0x4000 + (window - 0x4000);
                (linear < cartridge.rom_len() as u32).then_some(ROM_BASE + linear)
            }
            0xA000..=0xBFFF => {
                let linear = bank as u32 * 0x2000 + (window - 0xA000);
                (linear < cartridge.ram_len() as u32).then_some(RAM_BASE + linear)
            }
            0xC000..=0xCFFF if bank == 0 => {
                let linear = window - 0xC000;
                wram_len
                    .filter(|&len| (linear as usize) < len)
                    .map(|_| WRAM_BASE + linear)
            }
            0xD000..=0xDFFF => {
                let linear = bank as u32 * 0x1000 + (window - 0xD000);
                wram_len
                    .filter(|&len| (linear as usize) < len)
                    .map(|_| WRAM_BASE + linear)
            }
            0x8000..=0x9FFF => {
                let linear = bank as u32 * 0x2000 + (window - 0x8000);
                vram_len
                    .filter(|&len| linear < len)
                    .map(|_| VRAM_BASE + linear)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::tests::traced_program_console;
    use crate::debugger::{WatchCondition, cdl};
    use crate::isa::Sm83;
    use crate::{Console, Dmg};

    /// A four-bank MBC5 cart with 32 KB RAM, each ROM bank stamped with its
    /// index so a linear read reveals which bank a byte came from.
    fn mbc5_ram_cart() -> crate::cartridge::Cartridge {
        let mut rom = vec![0u8; 4 * 0x4000];
        for (i, bank) in rom.chunks_mut(0x4000).enumerate() {
            bank.fill(i as u8);
        }
        rom[0x147] = 0x1a; // MBC5 + RAM
        rom[0x149] = 3; // 32 KB (four 8 KB banks)
        crate::cartridge::Cartridge::new(rom, None)
    }

    #[test]
    fn sram_region_present_only_with_cart_ram() {
        let with_ram = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let regions = with_ram.memory_regions();
        let sram = regions.iter().find(|r| r.name == "sram").expect("sram");
        assert_eq!(sram.start, RAM_BASE);
        assert_eq!(sram.len, 4 * 0x2000);

        // traced_program_console is a plain no-RAM cart.
        let no_ram = Debugger::new(traced_program_console());
        assert!(no_ram.memory_regions().iter().all(|r| r.name != "sram"));
    }

    #[test]
    fn rom_region_spans_the_full_image() {
        let debugger = Debugger::new(traced_program_console());
        let rom = debugger
            .memory_regions()
            .into_iter()
            .find(|r| r.name == "rom")
            .expect("rom region");
        assert_eq!(rom.start, ROM_BASE);
        assert_eq!(rom.len, 0x8000);
    }

    #[test]
    fn synthetic_ram_peek_bypasses_bank_and_enable() {
        let mut cart = mbc5_ram_cart();
        cart.write(0x0000, 0x0A); // enable RAM
        cart.write(0x4000, 0x02); // RAM bank 2
        cart.write(0xA005, 0x77);
        cart.write(0x0000, 0x00); // disable RAM again
        let debugger = Debugger::new(Console::<Dmg>::new(cart, None));

        // The CPU bus sees the disabled RAM as open bus.
        assert_eq!(debugger.peek(0xA005), 0xFF);
        // The synthetic region reads the raw byte in bank 2 regardless.
        assert_eq!(debugger.peek(RAM_BASE + 2 * 0x2000 + 5), 0x77);
    }

    #[test]
    fn synthetic_rom_peek_reads_unmapped_bank() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        // File order, independent of what the mapper currently pages in.
        assert_eq!(debugger.peek(ROM_BASE), 0);
        assert_eq!(debugger.peek(ROM_BASE + 3 * 0x4000), 3);
    }

    #[test]
    fn present_and_locate_round_trip_over_synthetic_space() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let display = |a: u32| {
            let d = debugger.present_address(a);
            (d.bank, d.window, d.breakpoint)
        };
        // ROM bank 0 maps to the fixed $0000 window — an unambiguous breakpoint.
        assert_eq!(display(ROM_BASE + 0x0123), (Some(0), 0x0123, Some(0x0123)));
        assert_eq!(
            debugger.locate_bank_window(0, 0x0123),
            Some(ROM_BASE + 0x0123)
        );
        // ROM bank 3 maps to the switchable $4000 window — no breakpoint.
        let rom3 = 3 * 0x4000 + 0x0123;
        assert_eq!(display(ROM_BASE + rom3), (Some(3), 0x4123, None));
        assert_eq!(
            debugger.locate_bank_window(3, 0x4123),
            Some(ROM_BASE + rom3)
        );
        // SRAM bank 2 maps to the $A000 window.
        let sram2 = 2 * 0x2000 + 0x0055;
        assert_eq!(display(RAM_BASE + sram2), (Some(2), 0xA055, None));
        assert_eq!(
            debugger.locate_bank_window(2, 0xA055),
            Some(RAM_BASE + sram2)
        );
        // A pairing past the image, and a window in no synthetic region, reject.
        assert_eq!(debugger.locate_bank_window(99, 0x4000), None);
        assert_eq!(debugger.locate_bank_window(0, 0x8000), None);
        // ROM bank 0 presents only through the fixed $0000 window; a bank-0
        // pairing in the switchable $4000 window has no synthetic row.
        assert_eq!(debugger.locate_bank_window(0, 0x4123), None);
    }

    #[test]
    fn peek_past_a_synthetic_store_reads_open_bus() {
        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        let rom_len = debugger.game_boy().cartridge().rom_len() as u32;
        // One byte past the ROM image is in no store: open bus, not a truncated
        // re-read of the console bus.
        assert_eq!(debugger.peek(ROM_BASE + rom_len), 0xFF);
        // A synthetic address between the ROM and WRAM decades reads open bus.
        assert_eq!(debugger.peek(RAM_BASE + 0x00FF_FFFF), 0xFF);
    }

    /// A synthetic ROM anchor decodes the bytes of a bank the CPU bus does not
    /// currently page in — the whole point of the bank-complete space. The
    /// walk must preserve the anchor's high bits, not alias back onto the bus.
    #[test]
    fn synthetic_anchor_decodes_unmapped_bank_contents() {
        use missingno_core::disasm::{ReadMemory, Row, window_after};

        let debugger = Debugger::new(Console::<Dmg>::new(mbc5_ram_cart(), None));
        // The wake bank paged into $4000 is bank 1 (all 0x01); bank 3 is all
        // 0x03 and reachable only through the synthetic space.
        let anchor = ROM_BASE + 3 * 0x4000;
        assert_eq!(debugger.peek(anchor), 0x03);
        assert_eq!(debugger.peek(0x4000), 0x01);

        struct Peek<'a>(&'a Debugger<Dmg>);
        impl ReadMemory for Peek<'_> {
            fn read(&self, address: u32) -> u8 {
                self.0.peek(address)
            }
        }
        // 0x03 is INC BC (one byte), so rows step by one and carry the base.
        let rows = window_after(anchor, 3, &Sm83, &Peek(&debugger), None);
        assert_eq!(
            rows,
            vec![
                Row::Instruction(anchor),
                Row::Instruction(anchor + 1),
                Row::Instruction(anchor + 2),
            ]
        );
    }

    fn cart_with_ram(cart_type: u8, ram_size: u8) -> crate::cartridge::Cartridge {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = cart_type;
        rom[0x149] = ram_size;
        crate::cartridge::Cartridge::new(rom, None)
    }

    #[test]
    fn single_ram_bank_carts_match_sram_bank_zero() {
        // NoMbc+RAM, MBC2 and MBC7 carry exactly one RAM bank and no bank
        // register; a synthetic SRAM row composes a `{pc, sram-bank:0}` watch,
        // which must match while RAM is the mapped target rather than never fire.
        for (name, cart) in [
            ("NoMbc", cart_with_ram(0x08, 2)),
            ("MBC2", cart_with_ram(0x05, 0)),
            ("MBC7", cart_with_ram(0x22, 0)),
        ] {
            assert_eq!(cart.mapped_ram_bank(), Some(0), "{name}");
            let debugger = Debugger::new(Console::<Dmg>::new(cart, None));
            assert!(
                debugger.condition_matches(&WatchCondition::SramBank(0), &[]),
                "{name} sram-bank:0 watch should match"
            );
        }
    }

    #[test]
    fn mbc3_clock_mode_maps_no_ram_bank() {
        // MBC3 in RAM mode targets the selected bank; in clock mode it maps no
        // RAM, so the bank watch correctly does not match.
        let mut cart = cart_with_ram(0x10, 3);
        cart.write(0x4000, 0x02); // RAM mode, bank 2
        assert_eq!(cart.mapped_ram_bank(), Some(2));
        cart.write(0x4000, 0x08); // clock mode: seconds register
        assert_eq!(cart.mapped_ram_bank(), None);

        let debugger = Debugger::new(Console::<Dmg>::new(cart, None));
        assert!(!debugger.condition_matches(&WatchCondition::SramBank(0), &[]));
        assert!(!debugger.condition_matches(&WatchCondition::SramBank(2), &[]));
    }

    #[test]
    fn dmg_has_no_linear_wram_region() {
        let debugger = Debugger::new(traced_program_console());
        assert!(
            debugger
                .memory_regions()
                .iter()
                .all(|r| r.name != "wram-all")
        );
        // DMG's model exposes no bank-complete WRAM image.
        assert!(debugger.game_boy().model().wram_image().is_none());
    }

    #[test]
    fn dmg_has_no_linear_vram_region() {
        let debugger = Debugger::new(traced_program_console());
        assert!(
            debugger
                .memory_regions()
                .iter()
                .all(|r| r.name != "vram-all")
        );
        // DMG's single VRAM bank is fully visible through the $8000 window.
        assert!(debugger.game_boy().vram_image_len().is_none());
    }

    /// Code uploaded to work RAM and executed there disassembles live: the walk
    /// reads through peek and, since the code/data log never covers RAM, the
    /// backward context falls back to the heuristic sweep.
    #[test]
    fn disassembles_code_running_in_work_ram() {
        use missingno_core::disasm::{ReadMemory, Row, window_after};

        // Store a three-byte routine to $C000 (NOP; JR -3 → self-loop) then jump
        // to it, so the program counter ends up executing from WRAM.
        let mut rom = vec![0u8; 0x8000];
        let program = [
            0x3E, 0x00, // LD A,$00
            0xEA, 0x00, 0xC0, // LD ($C000),A
            0x3E, 0x18, // LD A,$18
            0xEA, 0x01, 0xC0, // LD ($C001),A
            0x3E, 0xFD, // LD A,$FD
            0xEA, 0x02, 0xC0, // LD ($C002),A
            0xC3, 0x00, 0xC0, // JP $C000
        ];
        rom[0x100..0x100 + program.len()].copy_from_slice(&program);
        let mut debugger = Debugger::new(Console::<Dmg>::new(
            crate::cartridge::Cartridge::new(rom, None),
            None,
        ));

        for _ in 0..64 {
            if debugger.pc() == 0xC000 {
                break;
            }
            debugger.step();
        }
        assert_eq!(debugger.pc(), 0xC000, "did not reach the WRAM routine");

        // The routine bytes are live in WRAM.
        assert_eq!(debugger.peek(0xC000), 0x00);
        assert_eq!(debugger.peek(0xC001), 0x18);
        assert_eq!(debugger.peek(0xC002), 0xFD);
        // The log records nothing for RAM addresses, so the disassembly's
        // backward context has no coverage and uses the heuristic.
        assert_eq!(debugger.cdl().flags(cdl::rom_offset(0xC000, None)), 0);

        struct Peek<'a>(&'a Debugger<Dmg>);
        impl ReadMemory for Peek<'_> {
            fn read(&self, address: u32) -> u8 {
                self.0.peek(address)
            }
        }
        let window = debugger
            .cdl()
            .window(0xC000, |address| cdl::rom_offset(address, None));
        let rows = window_after(0xC000, 2, &Sm83, &Peek(&debugger), Some(&window));
        assert_eq!(
            rows,
            vec![Row::Instruction(0xC000), Row::Instruction(0xC001)]
        );
    }
}
