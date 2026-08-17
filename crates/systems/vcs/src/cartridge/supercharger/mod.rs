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
//! # Standing in for the tape
//!
//! The deck, the PWM demodulation and the Starpath BIOS are not modelled. In
//! their place this takes the community "fastload" container — a snapshot of
//! what the BIOS would have streamed into RAM, one 8448-byte unit per tape
//! load — and runs its own small BIOS, written here, in the ROM bank: the two
//! documented entries ($F800 load, $F80A rewind-and-load) reach one loader that
//! asks the board for the unit the game named in $FA, then installs the state
//! the real BIOS leaves behind and jumps into the game through the $FA stub.
//!
//! Two consequences of the abstraction are worth naming. The accumulator the
//! real BIOS hands over carries tape-timing entropy; this hands over a fixed
//! byte, because a recording has to replay. And a rewind seeks tape position,
//! which nothing here has: both entries find a load by its id, so a container
//! whose units share an id only ever reaches the first of them.

mod bios;

use super::CartridgeError;

const BANK_SIZE: usize = 0x800;
const BANKS: usize = 3;
const BIOS_SIZE: usize = 0x800;
const PAGE_SIZE: usize = 0x100;
const MAX_PAGES: usize = BANKS * BANK_SIZE / PAGE_SIZE;

/// One tape load: the pages the BIOS streams into RAM, then its header.
pub const IMAGE_SIZE: usize = DATA_SIZE + PAGE_SIZE;
const DATA_SIZE: usize = 0x2000;

/// The most load units a container is read as. The largest known title carries
/// four; the rest of the range is slack.
pub const MAX_LOADS: usize = 8;

/// Whether a length is a fastload container: whole load units, up to the cap.
pub fn is_container(len: usize) -> bool {
    len.is_multiple_of(IMAGE_SIZE) && (1..=MAX_LOADS).contains(&(len / IMAGE_SIZE))
}

/// Header fields, from the header's own base.
const START: usize = 0;
const CONTROL: usize = 2;
const PAGE_COUNT: usize = 3;
const MULTILOAD: usize = 5;
const PAGE_TABLE: usize = 0x10;
const PAGE_CHECKSUMS: usize = 0x40;
/// The header's first eight bytes, its own checksum (byte 4) among them, are
/// summed against the container's total.
const HEADER_SUMMED: usize = 8;
/// Every header and page checksum settles its sum here.
const CHECKSUM_TOTAL: u8 = 0x55;

/// Any access here latches its low byte into the data-hold latch.
const LATCH_WINDOW: u16 = 0x1000;
/// A read here commits the data-hold latch into the control register.
const CONTROL_COMMIT: u16 = 0x1FF8;
/// The tape audio input. No deck is modelled, so the line reads low.
const TAPE_INPUT: u16 = 0x1FF9;
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

/// One load unit of the container: what the tape would have delivered.
struct Load {
    pages: Box<[u8]>,
    page_table: [u8; MAX_PAGES],
    page_count: usize,
    start: u16,
    control: u8,
    id: u8,
}

impl Load {
    /// Read one 8448-byte unit, refusing it if its checksums don't settle.
    fn parse(unit: usize, image: &[u8]) -> Result<Load, CartridgeError> {
        let header = &image[DATA_SIZE..];
        if sum(&header[..HEADER_SUMMED]) != CHECKSUM_TOTAL {
            return Err(CartridgeError::LoadChecksum { unit, page: None });
        }
        let page_count = usize::from(header[PAGE_COUNT]).min(MAX_PAGES);
        let mut page_table = [0u8; MAX_PAGES];
        page_table.copy_from_slice(&header[PAGE_TABLE..PAGE_TABLE + MAX_PAGES]);
        for page in 0..page_count {
            let data = &image[page * PAGE_SIZE..(page + 1) * PAGE_SIZE];
            let total = sum(data)
                .wrapping_add(page_table[page])
                .wrapping_add(header[PAGE_CHECKSUMS + page]);
            if total != CHECKSUM_TOTAL {
                return Err(CartridgeError::LoadChecksum {
                    unit,
                    page: Some(page),
                });
            }
        }
        Ok(Load {
            pages: image[..DATA_SIZE].into(),
            page_table,
            page_count,
            start: u16::from_le_bytes([header[START], header[START + 1]]),
            control: header[CONTROL],
            id: header[MULTILOAD],
        })
    }
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |total, &b| total.wrapping_add(b))
}

pub struct Supercharger {
    ram: [[u8; BANK_SIZE]; BANKS],
    bios: [u8; BIOS_SIZE],
    loads: Vec<Load>,
    /// Where the tape is standing: the unit the last load came from.
    current: usize,
    control: u8,
    data_hold: u8,
    pending: Option<PendingWrite>,
    transitions: u64,
    last_address: u16,
}

impl Supercharger {
    pub fn new(image: &[u8]) -> Result<Supercharger, CartridgeError> {
        let loads = image
            .chunks_exact(IMAGE_SIZE)
            .enumerate()
            .map(|(unit, chunk)| Load::parse(unit, chunk))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Supercharger {
            ram: [[0u8; BANK_SIZE]; BANKS],
            bios: bios::assemble(),
            loads,
            current: 0,
            // Power-on has to leave the BIOS answering the reset vector.
            control: 0,
            data_hold: 0,
            pending: None,
            transitions: 0,
            last_address: 0,
        })
    }

    /// The load the BIOS asked for: stuff its pages into RAM and leave the
    /// start address and control byte where the BIOS's stub template will find
    /// them. The answer is what the BIOS branches on.
    fn load(&mut self, requested: u8) -> u8 {
        let Some(index) = self.find(requested) else {
            return bios::NO_LOAD;
        };
        self.current = index;
        let Supercharger { ram, loads, .. } = self;
        let load = &loads[index];
        for page in 0..load.page_count {
            let entry = load.page_table[page];
            let bank = usize::from(entry & 0x03);
            let target = usize::from(entry >> 2 & 0x07) * PAGE_SIZE;
            if bank < BANKS {
                let source = page * PAGE_SIZE;
                ram[bank][target..target + PAGE_SIZE]
                    .copy_from_slice(&load.pages[source..source + PAGE_SIZE]);
            }
        }
        let [low, high] = load.start.to_le_bytes();
        let control = load.control;
        self.bios[bios::START_LOW] = low;
        self.bios[bios::START_HIGH] = high;
        self.bios[bios::CONTROL_BYTE] = control;
        bios::LOADED
    }

    /// The tape runs forward from where it stopped, so a request meets the next
    /// unit carrying that id; position only breaks ties between units sharing
    /// one.
    fn find(&self, requested: u8) -> Option<usize> {
        (0..self.loads.len())
            .map(|step| (self.current + step) % self.loads.len())
            .find(|&unit| self.loads[unit].id == requested)
    }

    fn configuration(&self) -> usize {
        usize::from(self.control >> 2 & 0x07)
    }

    /// Whether the BIOS is the thing the high window answers with.
    fn bios_mapped(&self) -> bool {
        matches!(HIGH[self.configuration()], HighWindow::Bios) && self.control & ROM_OFF == 0
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
        if address & 0x1FFF == TAPE_INPUT {
            return Some(0);
        }
        if self.bios_mapped() && address & 0x1F00 == bios::LOAD_REQUEST & 0x1F00 {
            return Some(self.load(address as u8));
        }
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

    /// The control register (bank configuration, RAM write-enable, ROM power),
    /// the data-hold latch, and where the tape is standing, for a state save.
    /// The in-flight pending write and the transition counter are transient and
    /// reset on restore.
    pub(super) fn bank_state(&self) -> Vec<u8> {
        vec![self.control, self.data_hold, self.current as u8]
    }

    pub(super) fn restore_bank_state(&mut self, bytes: &[u8]) {
        if let Some(&control) = bytes.first() {
            self.control = control;
        }
        if let Some(&data_hold) = bytes.get(1) {
            self.data_hold = data_hold;
        }
        if let Some(&current) = bytes.get(2) {
            self.current = usize::from(current).min(self.loads.len().saturating_sub(1));
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

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_CHECKSUM: usize = 4;

    /// A one-page load unit: `program` at `start`, mapped into `bank`, with the
    /// checksums the container requires.
    fn unit(id: u8, control: u8, start: u16, bank: u8, program: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; IMAGE_SIZE];
        image[..program.len()].copy_from_slice(program);
        let entry = ((start >> 8 & 0x07) as u8) << 2 | bank;

        let header = &mut image[DATA_SIZE..];
        header[START..START + 2].copy_from_slice(&start.to_le_bytes());
        header[CONTROL] = control;
        header[PAGE_COUNT] = 1;
        header[MULTILOAD] = id;
        header[PAGE_TABLE] = entry;
        let partial = sum(&header[..HEADER_SUMMED]);
        header[HEADER_CHECKSUM] = CHECKSUM_TOTAL.wrapping_sub(partial);

        let page = sum(&image[..PAGE_SIZE]);
        image[DATA_SIZE + PAGE_CHECKSUMS] = CHECKSUM_TOTAL.wrapping_sub(page).wrapping_sub(entry);
        image
    }

    fn container(units: &[Vec<u8>]) -> Vec<u8> {
        units.concat()
    }

    /// Two units under distinct ids, each one page of filler into bank 0.
    fn two_loads() -> Vec<u8> {
        container(&[
            unit(0, 0x04, 0xF100, 0, &[0x11; 4]),
            unit(5, 0x0C, 0xF200, 0, &[0x22; 4]),
        ])
    }

    fn read(ar: &mut Supercharger, address: u16) -> u8 {
        ar.read(address, 0).expect("the window answers")
    }

    /// Commit a control byte the way a program does: latch it, then strobe.
    fn configure(ar: &mut Supercharger, control: u8) {
        read(ar, 0xF000 | u16::from(control));
        read(ar, 0xFFF8);
    }

    #[test]
    fn a_multi_unit_container_stuffs_the_unit_the_request_names() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();

        assert_eq!(read(&mut ar, bios::LOAD_REQUEST + 5), bios::LOADED);
        assert_eq!(ar.ram[0][0x200], 0x22);
        assert_eq!(ar.bios[bios::START_LOW..=bios::START_HIGH], [0x00, 0xF2]);
        assert_eq!(ar.bios[bios::CONTROL_BYTE], 0x0C);

        assert_eq!(read(&mut ar, bios::LOAD_REQUEST), bios::LOADED);
        assert_eq!(ar.ram[0][0x100], 0x11);
        assert_eq!(ar.bios[bios::START_LOW..=bios::START_HIGH], [0x00, 0xF1]);
    }

    #[test]
    fn a_request_no_unit_answers_delivers_nothing() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();
        read(&mut ar, bios::LOAD_REQUEST);

        assert_eq!(read(&mut ar, bios::LOAD_REQUEST + 9), bios::NO_LOAD);
        assert_eq!(ar.ram[0][0x200], 0x00);
    }

    #[test]
    fn a_unit_whose_header_checksum_is_wrong_is_refused() {
        let mut image = two_loads();
        image[IMAGE_SIZE + DATA_SIZE + HEADER_CHECKSUM] ^= 0xFF;
        assert_eq!(
            Supercharger::new(&image).err(),
            Some(CartridgeError::LoadChecksum {
                unit: 1,
                page: None
            })
        );
    }

    #[test]
    fn a_unit_whose_page_checksum_is_wrong_is_refused() {
        let mut image = two_loads();
        image[0] ^= 0xFF;
        assert_eq!(
            Supercharger::new(&image).err(),
            Some(CartridgeError::LoadChecksum {
                unit: 0,
                page: Some(0),
            })
        );
    }

    #[test]
    fn the_write_lands_five_transitions_on_counting_repeats_once() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();
        configure(&mut ar, 0x06);

        read(&mut ar, 0xF047); // arm $47
        read(&mut ar, 0xF150); // nop opcode
        read(&mut ar, 0xF151); // nop's dummy read: the next opcode's address
        read(&mut ar, 0xF151); // ...which the next instruction re-reads
        read(&mut ar, 0xF152);
        read(&mut ar, 0xF153);
        assert_eq!(read(&mut ar, 0xF520), 0x47);
        assert_eq!(ar.ram[0][0x520], 0x47);
    }

    #[test]
    fn a_read_of_the_latch_page_corrupts_ram_when_writes_are_enabled() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();
        configure(&mut ar, 0x06);

        read(&mut ar, 0xF042); // an innocent-looking load
        read(&mut ar, 0xF150);
        read(&mut ar, 0xF151);
        read(&mut ar, 0xF152);
        read(&mut ar, 0xF153);
        read(&mut ar, 0xF520);
        assert_eq!(ar.ram[0][0x520], 0x42);
    }

    #[test]
    fn the_strobe_commits_a_latch_of_any_age() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();
        configure(&mut ar, 0x00);

        read(&mut ar, 0xF014);
        for address in 0xF200..0xF280 {
            read(&mut ar, address);
        }
        read(&mut ar, 0xFFF8);
        assert_eq!(ar.control, 0x14);
    }

    #[test]
    fn a_write_whose_transition_falls_outside_mapped_ram_is_refused() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();
        configure(&mut ar, 0x06);

        let bios_byte = ar.peek(0xFF00).unwrap();
        read(&mut ar, 0xF0AB);
        for address in 0xF150..0xF154 {
            read(&mut ar, address);
        }
        assert_eq!(read(&mut ar, 0xFF00), bios_byte);
        assert_eq!(ar.peek(0xFF00), Some(bios_byte));

        // The same shape with the fifth transition off the cart entirely.
        let banks = ar.ram;
        read(&mut ar, 0xF0AB);
        for address in 0xF150..0xF154 {
            read(&mut ar, address);
        }
        assert_eq!(ar.read(0x0080, 0), None);
        assert_eq!(ar.ram, banks);
    }

    #[test]
    fn a_read_of_the_tape_input_leaves_the_latch_alone() {
        let mut ar = Supercharger::new(&two_loads()).unwrap();

        read(&mut ar, 0xF03C);
        assert_eq!(read(&mut ar, 0xFFF9), 0x00);
        read(&mut ar, 0xFFF8);
        assert_eq!(ar.control, 0x3C);
    }
}
