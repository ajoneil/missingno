//! The Starpath Supercharger: not a ROM board at all, but 6 KB of RAM in three
//! 2 KB banks plus a 2 KB BIOS, loaded from cassette tape.
//!
//! The cart edge carries no write line — the 6507's R/W never reaches it — so
//! *everything* the board does is driven by READS:
//!
//! * A read anywhere in $F000-$F0FF latches that read's low address byte into
//!   the data-hold latch. A later read of $FFF8 commits it into the control
//!   register (bank configuration, RAM write enable, ROM power). There is no
//!   upper limit on the gap between the two.
//! * With write enable armed, that same latched byte is written to RAM — and
//!   forced onto the data bus, so the CPU reads it too — at the cart access
//!   exactly five address-bus *transitions* later. A transition is any cycle
//!   whose address differs from the previous cycle's, so an instruction that
//!   reads one address twice back to back counts once.
//!
//! The consequence kevtris states outright: with writes enabled you cannot read
//! from $F0xx without corrupting RAM. An innocent `lda $F042` clobbers whatever
//! RAM cell is accessed five transitions later.
//!
//! Tape audio is not modelled. Like every emulator this takes the community
//! "fastload" image — a snapshot of what the BIOS would have streamed into RAM,
//! plus the header it writes when a tape finishes — and stands in a BIOS that
//! only launches it.

const BANK_SIZE: usize = 0x800;
const BANKS: usize = 3;
const BIOS_SIZE: usize = 0x800;
const PAGE_SIZE: usize = 0x100;

/// The fastload container: the RAM image, a BIOS placeholder, then the header.
pub const IMAGE_SIZE: usize = 0x2100;
const HEADER: usize = 0x2000;
const PAGE_TABLE: usize = HEADER + 16;

/// Any access here latches its low byte into the data-hold latch.
const LATCH_WINDOW: u16 = 0x1000;
/// A read here commits the data-hold latch into the control register.
const CONTROL_COMMIT: u16 = 0x1FF8;
/// The window's halves; the low one is always RAM.
const HIGH_WINDOW: u16 = 0x1800;

/// Address-bus transitions from the arming read to the write landing.
const WRITE_DELAY: u64 = 5;

/// Control register bits.
const WRITE_ENABLE: u8 = 0x02;
const ROM_OFF: u8 = 0x01;

/// What the high window answers with, per bank configuration (control D4-D2).
enum HighWindow {
    Bios,
    Ram(usize),
}

/// The bank the low window shows, per configuration. Banks 0 and 1 are never
/// both mapped.
const LOW_BANK: [usize; 8] = [2, 0, 2, 0, 2, 1, 2, 1];
const HIGH: [HighWindow; 8] = [
    HighWindow::Bios,
    HighWindow::Bios,
    HighWindow::Ram(0),
    HighWindow::Ram(2),
    HighWindow::Bios,
    HighWindow::Bios,
    HighWindow::Ram(1),
    HighWindow::Ram(2),
];

/// A write armed by a $F0xx read, waiting for its transition to come round.
struct PendingWrite {
    value: u8,
    at_transition: u64,
}

pub struct Ar {
    ram: [[u8; BANK_SIZE]; BANKS],
    bios: [u8; BIOS_SIZE],
    control: u8,
    data_hold: u8,
    pending: Option<PendingWrite>,
    transitions: u64,
    last_address: u16,
}

impl Ar {
    pub fn new(image: &[u8]) -> Ar {
        let mut ram = [[0u8; BANK_SIZE]; BANKS];
        // Stand in for the BIOS's tape load: the header's page table says where
        // each 256-byte page of the image belongs in RAM.
        let pages = usize::from(image[HEADER + 3]).min(BANKS * BANK_SIZE / PAGE_SIZE);
        for page in 0..pages {
            let entry = image[PAGE_TABLE + page];
            let bank = usize::from(entry & 0x03);
            let target = usize::from(entry >> 2 & 0x07) * PAGE_SIZE;
            if bank < BANKS {
                let source = page * PAGE_SIZE;
                ram[bank][target..target + PAGE_SIZE]
                    .copy_from_slice(&image[source..source + PAGE_SIZE]);
            }
        }

        let start = u16::from_le_bytes([image[HEADER], image[HEADER + 1]]);
        Ar {
            ram,
            bios: Ar::launcher(start),
            // The header's control byte is the register the BIOS leaves behind.
            control: image[HEADER + 2],
            data_hold: 0,
            pending: None,
            transitions: 0,
            last_address: 0,
        }
    }

    /// A stand-in BIOS: the real one streams the tape and jumps to the loaded
    /// program, and only that last step is left to do. It sits clear of the
    /// $FFF8 commit hotspot.
    fn launcher(start: u16) -> [u8; BIOS_SIZE] {
        let mut bios = [0u8; BIOS_SIZE];
        let [low, high] = start.to_le_bytes();
        bios[0..3].copy_from_slice(&[0x4C, low, high]);
        // The reset vector, at the top of the window, points at that jump.
        bios[BIOS_SIZE - 4..BIOS_SIZE - 2].copy_from_slice(&[0x00, 0xF8]);
        bios
    }

    fn configuration(&self) -> usize {
        usize::from(self.control >> 2 & 0x07)
    }

    /// The RAM cell an address reaches, unless the address lands on a BIOS the
    /// configuration has mapped high.
    fn cell(&self, address: u16) -> Option<(usize, usize)> {
        let offset = usize::from(address & 0x7FF);
        match address & 0x1FFF < HIGH_WINDOW {
            true => Some((LOW_BANK[self.configuration()], offset)),
            false => match HIGH[self.configuration()] {
                HighWindow::Ram(bank) => Some((bank, offset)),
                HighWindow::Bios => None,
            },
        }
    }

    /// The delay counts transitions of the whole address bus, not just the
    /// cart's own accesses — an instruction fetching a pointer from zero page
    /// walks it as surely as a read of the window does. A cycle repeating the
    /// previous address is no transition at all.
    fn cycle(&mut self, address: u16) {
        if address != self.last_address {
            self.transitions += 1;
            self.last_address = address;
        }
    }

    /// The write commits when its transition comes round; the board gives up
    /// once the moment has passed.
    fn commit_write(&mut self, address: u16) -> Option<u8> {
        let pending = self.pending.as_ref()?;
        if self.transitions < pending.at_transition {
            return None;
        }
        let value = pending.value;
        let due = self.transitions == pending.at_transition;
        self.pending = None;
        if !due {
            return None;
        }
        // Write enable has to still be standing when the write lands, and it
        // needs a RAM cell to land in: a config committed during the delay
        // disarms the write it was itself armed by.
        if self.control & WRITE_ENABLE == 0 {
            return None;
        }
        let (bank, offset) = self.cell(address)?;
        self.ram[bank][offset] = value;
        // The latch drives the bus, so the committing read sees it too.
        Some(value)
    }

    /// A $F0xx access loads the data-hold latch. With writes enabled the latch
    /// freezes while one is pending, so the first armed value wins; with them
    /// disabled it is transparent and reloads on every access.
    fn latch(&mut self, address: u16, write_in_flight: bool) {
        let writes_on = self.control & WRITE_ENABLE != 0;
        if writes_on && write_in_flight {
            return;
        }
        self.data_hold = address as u8;
        if writes_on {
            self.pending = Some(PendingWrite {
                value: self.data_hold,
                at_transition: self.transitions + WRITE_DELAY,
            });
        }
    }

    /// A cart access: land a due write, then latch and commit. Everything the
    /// board does is read-driven, and it cannot tell a store from a load, so a
    /// write cycle drives exactly the same decode — only the CPU's byte goes
    /// nowhere.
    fn window_access(&mut self, address: u16) -> Option<u8> {
        // The freeze is decided by the write still being in flight as the
        // access begins: the very access that commits one cannot also arm the
        // next, even though it lands in the latch window.
        let write_in_flight = self.pending.is_some();
        let driven = self.commit_write(address);
        if address & 0x1F00 == LATCH_WINDOW {
            self.latch(address, write_in_flight);
        }
        if address & 0x1FFF == CONTROL_COMMIT {
            self.control = self.data_hold;
            // Committing a config abandons any write still in flight, so the
            // latch read that set the config up cannot go on to corrupt RAM.
            self.pending = None;
        }
        driven
    }

    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        self.cycle(address);
        if !super::selects_window(address) {
            return None;
        }
        let driven = self.window_access(address);
        Some(driven.unwrap_or_else(|| self.peek(address).unwrap_or(residue)))
    }

    pub fn write_access(&mut self, address: u16) {
        self.cycle(address);
        if super::selects_window(address) {
            self.window_access(address);
        }
    }

    /// The Supercharger's tape-loaded RAM banks as one linear space.
    pub(super) fn ram(&self) -> &[u8] {
        self.ram.as_flattened()
    }

    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        self.ram.as_flattened_mut()
    }

    /// The control register (bank configuration, RAM write-enable, ROM power)
    /// and the data-hold latch, for a state save. The in-flight pending write
    /// and the transition counter are transient and reset on restore.
    pub(super) fn bank_state(&self) -> Vec<u8> {
        vec![self.control, self.data_hold]
    }

    pub(super) fn restore_bank_state(&mut self, bytes: &[u8]) {
        if let Some(&control) = bytes.first() {
            self.control = control;
        }
        if let Some(&data_hold) = bytes.get(1) {
            self.data_hold = data_hold;
        }
        self.pending = None;
    }

    /// `None` where the powered-down BIOS leaves the window floating.
    pub fn peek(&self, address: u16) -> Option<u8> {
        match self.cell(address) {
            Some((bank, offset)) => Some(self.ram[bank][offset]),
            None => match self.control & ROM_OFF != 0 {
                true => None,
                false => Some(self.bios[usize::from(address & 0x7FF)]),
            },
        }
    }
}
