//! The board's stores seen whole, past the window the 6507 gets: every bank of
//! the ROM image in file order and all of the cart RAM linearised. The debugger
//! reads its bank-complete regions from here, and a state save/restore captures
//! the selection that decides which slice of them the window shows.

use super::{Board, Cartridge};

impl Cartridge {
    /// The board's cart RAM as one linear space, for the debugger's
    /// bank-complete region. Empty for a board with no RAM the core stores
    /// accessibly.
    fn ram_slice(&self) -> &[u8] {
        match &self.board {
            Board::Atari(board) => board.ram(),
            Board::Fa(board) => board.ram(),
            Board::Cv(board) => board.ram(),
            Board::ThreeE(board) => board.ram(),
            Board::ThreeEPlus(board) => board.ram(),
            Board::Ar(board) => board.ram(),
            Board::Wd(board) => board.ram(),
            // E7's two RAM stores (the 1 KB low bank and the 256-byte high bank)
            // don't linearise into one slice, so it stays out of this region.
            Board::E7(_) => &[],
            // Boards with no core-stored cart RAM.
            Board::Empty
            | Board::Plain(_)
            | Board::E0(_)
            | Board::Dpc(_)
            | Board::F0(_)
            | Board::Jane(_)
            | Board::Wf8(_)
            | Board::Fc(_)
            | Board::ZeroFa0(_)
            | Board::Zero3E0(_)
            | Board::Fe(_)
            | Board::Ua(_)
            | Board::ThreeF(_)
            | Board::Sb(_)
            | Board::Zero840(_)
            | Board::X07(_)
            | Board::Mdm(_) => &[],
        }
    }

    /// The cart RAM size in bytes; zero when the board exposes none.
    pub fn ram_len(&self) -> usize {
        self.ram_slice().len()
    }

    /// The board's linear cart RAM as a writable slice, for a state restore.
    /// Mirrors [`ram_slice`](Self::ram_slice); empty for a board with none.
    fn ram_slice_mut(&mut self) -> &mut [u8] {
        match &mut self.board {
            Board::Atari(board) => board.ram_mut(),
            Board::Fa(board) => board.ram_mut(),
            Board::Cv(board) => board.ram_mut(),
            Board::ThreeE(board) => board.ram_mut(),
            Board::ThreeEPlus(board) => board.ram_mut(),
            Board::Ar(board) => board.ram_mut(),
            Board::Wd(board) => board.ram_mut(),
            _ => &mut [],
        }
    }

    /// Restore linear cart RAM from a saved span, up to the board's RAM size.
    pub fn restore_ram(&mut self, bytes: &[u8]) {
        let ram = self.ram_slice_mut();
        let len = ram.len().min(bytes.len());
        ram[..len].copy_from_slice(&bytes[..len]);
    }

    /// A side-effect-free read of linearised cart RAM at `offset`; `0xFF` past
    /// the end.
    pub fn peek_ram(&self, offset: usize) -> u8 {
        self.ram_slice().get(offset).copied().unwrap_or(0xff)
    }

    /// The board's full ROM image, all banks in file order, for the debugger's
    /// bank-complete region. Empty for a board with no retained image (a plain
    /// board is already fully visible; the Supercharger keeps only tape RAM).
    fn rom_slice(&self) -> &[u8] {
        match &self.board {
            Board::Atari(board) => board.rom(),
            Board::Fa(board) => board.rom(),
            Board::E0(board) => board.rom(),
            Board::E7(board) => board.rom(),
            Board::Dpc(board) => board.rom(),
            Board::F0(board) => board.rom(),
            Board::Jane(board) => board.rom(),
            Board::Wf8(board) => board.rom(),
            Board::Wd(board) => board.rom(),
            Board::Fc(board) => board.rom(),
            Board::ZeroFa0(board) => board.rom(),
            Board::Zero3E0(board) => board.rom(),
            Board::Ua(board) => board.rom(),
            Board::ThreeF(board) => board.rom(),
            Board::ThreeE(board) => board.rom(),
            Board::ThreeEPlus(board) => board.rom(),
            Board::Fe(board) => board.rom(),
            Board::Sb(board) => board.rom(),
            Board::Zero840(board) => board.rom(),
            Board::X07(board) => board.rom(),
            Board::Mdm(board) => board.rom(),
            // Boards with no retained image: an empty slot, a plain board that
            // is already fully visible through the window, the Supercharger
            // (tape RAM only), and CommaVid (its 2 KB is RAM, not banked ROM).
            Board::Empty | Board::Plain(_) | Board::Ar(_) | Board::Cv(_) => &[],
        }
    }

    /// The board's ROM image size in bytes; zero when it retains none.
    pub fn rom_len(&self) -> usize {
        self.rom_slice().len()
    }

    /// A side-effect-free read of the linearised ROM image at `offset`,
    /// independent of the current bank; `0xFF` past the end.
    pub fn peek_rom(&self, offset: usize) -> u8 {
        self.rom_slice().get(offset).copied().unwrap_or(0xff)
    }

    /// The 4 KB bank paged into the cart window, on boards that keep one; `None`
    /// for an unbanked board.
    pub fn selected_bank(&self) -> Option<usize> {
        self.board.selected_bank()
    }

    /// The board's durable bank/slot selection as an opaque blob, for a state
    /// save — the multi-slot boards a single [`selected_bank`](Self::selected_bank)
    /// cannot describe (three windows, a lower-window ROM/RAM select, four
    /// independently-banked segments). Empty for a board whose whole selection is
    /// the single bank, or none at all.
    pub fn bank_state(&self) -> Vec<u8> {
        match &self.board {
            Board::Fa(board) => board.bank_state(),
            Board::E0(board) => board.bank_state(),
            Board::E7(board) => board.bank_state(),
            Board::Dpc(board) => board.bank_state(),
            Board::Ar(board) => board.bank_state(),
            Board::Wd(board) => board.bank_state(),
            Board::ThreeE(board) => board.bank_state(),
            Board::ThreeEPlus(board) => board.bank_state(),
            _ => Vec::new(),
        }
    }

    /// Restore a board's bank/slot selection from a saved blob — the inverse of
    /// [`bank_state`](Self::bank_state). Transient switch-delay and arm latches
    /// are not reconstructed (a Tier-2a limit). A no-op for a board with none.
    pub fn restore_bank_state(&mut self, bytes: &[u8]) {
        match &mut self.board {
            Board::Fa(board) => board.restore_bank_state(bytes),
            Board::E0(board) => board.restore_bank_state(bytes),
            Board::E7(board) => board.restore_bank_state(bytes),
            Board::Dpc(board) => board.restore_bank_state(bytes),
            Board::Ar(board) => board.restore_bank_state(bytes),
            Board::Wd(board) => board.restore_bank_state(bytes),
            Board::ThreeE(board) => board.restore_bank_state(bytes),
            Board::ThreeEPlus(board) => board.restore_bank_state(bytes),
            _ => {}
        }
    }

    /// Re-page a banked board to a saved bank. A boardʼs extra switch state
    /// beyond its selected bank (a one-way lock, a pending half-latch) is not
    /// reconstructed — a Tier-2a limit for those exotic boards. `None` and an
    /// unbanked board are no-ops.
    pub fn restore_bank(&mut self, bank: Option<usize>) {
        let Some(bank) = bank else { return };
        match &mut self.board {
            Board::Atari(board) => board.set_bank(bank),
            Board::F0(board) => board.set_bank(bank),
            Board::Jane(board) => board.set_bank(bank),
            Board::Wf8(board) => board.set_bank(bank),
            Board::Fc(board) => board.set_bank(bank),
            Board::Sb(board) => board.set_bank(bank),
            Board::Mdm(board) => board.set_bank(bank),
            Board::X07(board) => board.set_bank(bank),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CartType, Cartridge, DumpFit};

    const CLOCK: f32 = 3_579_545.0;

    #[test]
    fn rom_slice_is_the_full_image_in_file_order() {
        let mut rom = vec![0u8; 0x2000];
        for (i, bank) in rom.chunks_mut(0x1000).enumerate() {
            bank.fill(i as u8);
        }
        let cart = Cartridge::load(&rom, Some(CartType::F8), CLOCK, DumpFit::Exact).unwrap();
        assert_eq!(cart.rom_len(), 0x2000);
        // Both banks readable regardless of which is paged in.
        assert_eq!(cart.peek_rom(0), 0);
        assert_eq!(cart.peek_rom(0x1000), 1);
        assert_eq!(cart.peek_rom(0x2000), 0xff); // past the end
    }

    #[test]
    fn superchip_ram_round_trips_through_the_linear_view() {
        let cart_image = vec![0u8; 0x2000];
        let mut cart =
            Cartridge::load(&cart_image, Some(CartType::F8Sc), CLOCK, DumpFit::Exact).unwrap();
        assert_eq!(cart.ram_len(), 0x80);
        // The Superchip write port is the low half of the window.
        cart.write_access(0x1000, 0x77, 0);
        assert_eq!(cart.peek_ram(0), 0x77);
        assert_eq!(cart.peek_ram(0x80), 0xff); // past the end
    }

    #[test]
    fn wd_board_exposes_its_scratch_ram() {
        let cart = Cartridge::load(
            &vec![0u8; 0x2000],
            Some(CartType::Wd),
            CLOCK,
            DumpFit::Exact,
        )
        .unwrap();
        // The Wickstead board's 64-byte scratch RAM is a plainly accessible
        // store, so it contributes a bank-complete `cart ram` region.
        assert_eq!(cart.ram_len(), 0x40);
        assert_eq!(cart.peek_ram(0x40), 0xff); // past the end
    }

    #[test]
    fn fa_bank_state_round_trips_a_non_default_bank() {
        // The FA (CBS RAM Plus) board pages a bank the single `selected_bank`
        // field never described, so its bank was lost across a save. Stamp each
        // 4 KiB bank, switch to bank 2, and confirm the bank-state blob restores
        // it into a fresh board.
        let mut rom = vec![0u8; 0x3000];
        for (i, bank) in rom.chunks_mut(0x1000).enumerate() {
            bank.fill(i as u8);
        }
        let mut cart = Cartridge::load(&rom, Some(CartType::Fa), CLOCK, DumpFit::Exact).unwrap();
        // Read above the 512-byte cart-RAM window at the base of the window, so
        // the byte comes from the paged ROM bank.
        const ROM_READ: u16 = 0x1200;
        assert_eq!(cart.peek(ROM_READ), 0, "powers on at bank 0");

        // Hotspot $1FFA with the data bus D0 set pages in bank 2.
        cart.write_access(0x1FFA, 0x01, 0);
        assert_eq!(cart.peek(ROM_READ), 2);
        let state = cart.bank_state();
        assert_eq!(state, vec![2u8], "the selected bank is captured");

        let mut restored =
            Cartridge::load(&rom, Some(CartType::Fa), CLOCK, DumpFit::Exact).unwrap();
        assert_eq!(restored.peek(ROM_READ), 0);
        restored.restore_bank_state(&state);
        assert_eq!(restored.peek(ROM_READ), 2, "the FA bank restores");
    }

    #[test]
    fn a_plain_board_exposes_no_synthetic_stores() {
        let cart = Cartridge::load(
            &vec![0u8; 0x1000],
            Some(CartType::Plain4K),
            CLOCK,
            DumpFit::Exact,
        )
        .unwrap();
        // A plain board is fully visible through the window, so it contributes
        // no synthetic ROM or RAM region.
        assert_eq!(cart.ram_len(), 0);
        assert_eq!(cart.rom_len(), 0);
    }
}
