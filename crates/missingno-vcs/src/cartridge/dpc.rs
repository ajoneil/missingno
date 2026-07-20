//! The DPC ("Display Processor Chip"): the custom NMOS part in Pitfall II.
//!
//! The chip sits alongside an 8 KB F8-banked program ROM whose chip-enable it
//! gates, and carries its own 2 KB display ROM that the CPU cannot address
//! directly. Instead it reads through eight address generators — the fetchers —
//! plus a random-number generator, three music voices and a line-drawing adder.
//! US Patent 4,644,495 (Crane, Activision) is the primary description; where it
//! and the emulators disagree, this models the patent.
//!
//! It has no clock pin and no R/W line, so every function is decoded from the
//! address alone. Two consequences fall out of that and are modelled here even
//! though no emulator implements them: register direction comes from A6 alone,
//! so a CPU *write* to a read window still fires the read decoders (and steps a
//! fetcher), and the 24-pin part does not decode A11, so the register file
//! answers a second time at $1800-$187F. Both are patent/kevtris-derived and
//! await a socketed board to confirm.

/// A read-only view of the DPC's internals for the debugger's Cartridge section.
#[derive(Clone, Debug)]
pub struct DpcView {
    pub fetchers: [DpcFetcherView; FETCHERS],
    pub rng: u8,
    pub bank: usize,
}

/// One data fetcher's state.
#[derive(Clone, Debug)]
pub struct DpcFetcherView {
    /// The 11-bit down-counter into the display ROM.
    pub counter: u16,
    pub top: u8,
    pub bottom: u8,
    /// The equality comparator's output — the voice's square wave in music mode.
    pub flag: bool,
    pub music: bool,
    /// The voice clocks from the RC oscillator rather than its read strobe.
    pub oscillator: bool,
}

const BANK_SIZE: usize = 0x1000;
const PROGRAM_SIZE: usize = 0x2000;
const DISPLAY_SIZE: usize = 0x800;
/// The program and display ROMs; anything past them is the dumper's tail.
pub const IMAGE_SIZE: usize = PROGRAM_SIZE + DISPLAY_SIZE;
const FETCHERS: usize = 8;
/// The first fetcher with a music voice; DF5-DF7 have one.
const FIRST_MUSIC_FETCHER: usize = 5;
/// The fetcher carrying the draw-line adder.
const DRAW_LINE_FETCHER: usize = 4;

/// The register file selects on A12 with A10-A7 low — and A11 undecoded, which
/// is what mirrors it into $1800-$187F.
const REGISTER_DECODE: u16 = 0x1780;
const REGISTER_SELECT: u16 = 0x1000;
/// Register direction is A6 alone: low is a read function, high a write one.
const WRITE_FUNCTION: u16 = 0x40;

const F8_HOTSPOT_BASE: u16 = 0x1FF8;
const BANKS: usize = 2;

/// SIN5/SIN6/SIN7 sum through a fixed 4/5/6 weighting, indexed {DF7,DF6,DF5}.
const AMPLITUDE: [u8; 8] = [0, 4, 5, 9, 6, 10, 11, 15];

/// The music oscillator, an RC on the cart board: a 560K resistor against a cap
/// on the die, free-running and owing nothing to the console's clock. Its pitch
/// is therefore per-cart and drifts — kevtris likens a real one to a theremin —
/// so no exact figure exists to model. The patent asks for 42 kHz and allows
/// 15-80 kHz; every independent estimate of shipped silicon lands at about half
/// the patent's figure, clustering near this one, and only the ratio
/// OSC/(Top+1) is specified. Voices clocked from a fetcher's read strobe
/// instead never consult it.
const OSCILLATOR_HZ: f32 = 20_000.0;

/// An 11-bit down-counter over the display ROM, with an equality-driven flag.
/// In music mode the low stage instead free-runs as an 8-bit down-counter.
struct Fetcher {
    counter: u16,
    top: u8,
    bottom: u8,
    flag: bool,
    music: bool,
    /// The voice clocks from the internal RC oscillator rather than the
    /// fetcher's own read strobe.
    oscillator_clocked: bool,
}

impl Fetcher {
    fn new() -> Fetcher {
        Fetcher {
            counter: 0,
            top: 0,
            bottom: 0,
            flag: false,
            music: false,
            oscillator_clocked: false,
        }
    }

    fn low(&self) -> u8 {
        self.counter as u8
    }

    /// The comparators are continuous: the instant the counter equals Top the
    /// set input fires, with no read needed. Top and Bottom equal at once
    /// leaves the set input winning.
    fn evaluate_flag(&mut self) {
        if self.low() == self.top {
            self.flag = true;
        } else if self.low() == self.bottom {
            self.flag = false;
        }
    }

    /// One read strobe. A music voice's low stage reloads from Top past zero,
    /// giving a period of Top+1; otherwise the whole 11 bits count down and
    /// wrap.
    fn clock(&mut self) {
        if !self.music {
            self.counter = self.counter.wrapping_sub(1) & 0x7FF;
            return;
        }
        // The clock select hands this voice to the oscillator instead, and the
        // read strobe then moves nothing.
        if !self.oscillator_clocked {
            self.clock_music();
        }
    }

    /// One clock of a music voice: the low stage counts down and reloads from
    /// Top past zero, so the voice's period is Top+1 clocks. The comparators
    /// watch it the whole way, which is what turns the flag into a square wave
    /// — nothing need read the voice for its note to advance.
    fn clock_music(&mut self) {
        let low = match self.low() {
            0 => self.top,
            low => low - 1,
        };
        self.counter = (self.counter & 0x700) | u16::from(low);
        self.evaluate_flag();
    }

    /// The voice's square-wave output.
    fn square_wave(&self) -> bool {
        self.music && self.flag
    }
}

/// The patent's adder pair: $1004/$1005 reads pulse an add of DF4's Top into a
/// latch, and the carry out becomes DLC.
struct DrawLine {
    enabled: bool,
    latch: u8,
    carry: bool,
    /// Loaded from D7-D4 of a $1060-$1067 write; masks the carry into the high
    /// nibble of the amplitude reads.
    movamt: u8,
}

pub struct Dpc {
    program: Vec<u8>,
    display: Vec<u8>,
    bank: usize,
    fetchers: [Fetcher; FETCHERS],
    rng: u8,
    draw_line: DrawLine,
    /// The oscillator's period, in the console clocks the board is stepped by,
    /// and how far through one it is.
    oscillator_period: f32,
    oscillator_phase: f32,
}

impl Dpc {
    pub fn new(rom: &[u8], clock_hz: f32) -> Dpc {
        Dpc {
            program: rom[..PROGRAM_SIZE].to_vec(),
            display: rom[PROGRAM_SIZE..PROGRAM_SIZE + DISPLAY_SIZE].to_vec(),
            bank: 0,
            fetchers: std::array::from_fn(|_| Fetcher::new()),
            rng: 0,
            draw_line: DrawLine {
                enabled: false,
                latch: 0,
                carry: false,
                movamt: 0,
            },
            oscillator_period: clock_hz / OSCILLATOR_HZ,
            oscillator_phase: 0.0,
        }
    }

    /// One console clock. The oscillator free-runs against it, so a voice the
    /// clock select handed to the oscillator advances here rather than on a
    /// read — which is the only way a voice nothing reads can play at all.
    pub fn tick(&mut self) {
        self.oscillator_phase += 1.0;
        if self.oscillator_phase < self.oscillator_period {
            return;
        }
        self.oscillator_phase -= self.oscillator_period;
        for voice in &mut self.fetchers[FIRST_MUSIC_FETCHER..] {
            if voice.music && voice.oscillator_clocked {
                voice.clock_music();
            }
        }
    }

    /// An 8-bit LFSR shifting left, with bit 0 fed by the XNOR of bits 3, 4, 5
    /// and 7. All-ones is an absorbing lock-up; every other value has period
    /// 255. It clocks on every chip select — the patent's decode — so opcode
    /// fetches from the program ROM step it too.
    fn clock_rng(&mut self) {
        let taps = (self.rng >> 3) ^ (self.rng >> 4) ^ (self.rng >> 5) ^ (self.rng >> 7);
        self.rng = (self.rng << 1) | (!taps & 1);
    }

    fn is_register(address: u16) -> bool {
        address & REGISTER_DECODE == REGISTER_SELECT
    }

    fn hotspot(&mut self, address: u16) {
        let offset = (address & 0x1FFF).wrapping_sub(F8_HOTSPOT_BASE) as usize;
        if offset < BANKS {
            self.bank = offset;
        }
    }

    /// The display ROM is addressed inverted with respect to the image.
    fn display_byte(&self, fetcher: &Fetcher) -> u8 {
        self.display[DISPLAY_SIZE - 1 - (fetcher.counter & 0x7FF) as usize]
    }

    /// The low nibble of an amplitude read: the three voices through the mixer.
    fn mixed_amplitude(&self) -> u8 {
        let voice = |n: usize| u8::from(self.fetchers[n].square_wave());
        AMPLITUDE[usize::from(voice(5) | voice(6) << 1 | voice(7) << 2)]
    }

    /// The high nibble: MOVAMT gated by the draw-line carry.
    fn movement(&self) -> u8 {
        match self.draw_line.carry {
            true => self.draw_line.movamt << 4,
            false => 0,
        }
    }

    fn pulse_draw_line(&mut self) {
        if self.draw_line.enabled {
            let (latch, carry) = self.draw_line.latch.overflowing_add(self.fetchers[4].top);
            self.draw_line.latch = latch;
            self.draw_line.carry = carry;
        }
    }

    /// A read function, decoded from A5-A3 with the fetcher in A2-A0. Anything
    /// above the RNG/music window strobes the addressed fetcher: the flag is
    /// re-evaluated from the current counter, the byte formed, and only then
    /// does the pointer step.
    fn read_function(&mut self, offset: u16) -> u8 {
        let index = usize::from(offset & 0x07);
        let window = (offset >> 3) & 0x07;
        if window == 0 {
            // The RNG/music window belongs to no fetcher and moves none.
            return match index {
                0..=3 => self.rng,
                _ => {
                    // $004/$005 pulse the adder; $006/$007 report it as it is.
                    if index < 6 {
                        self.pulse_draw_line();
                    }
                    self.movement() | self.mixed_amplitude()
                }
            };
        }

        self.fetchers[index].evaluate_flag();
        let data = self.display_byte(&self.fetchers[index]);
        let flag = self.fetchers[index].flag;
        let masked = match flag {
            true => data,
            false => 0,
        };
        let value = match window {
            1 => data,
            2 => masked,
            // The permuting and shifting windows transform the masked byte, so
            // a clear flag yields zero through all of them.
            3 => masked.rotate_left(4),
            4 => masked.reverse_bits(),
            5 => masked >> 1,
            6 => masked << 1,
            _ => match flag {
                true => 0xFF,
                false => 0x00,
            },
        };
        self.fetchers[index].clock();
        value
    }

    /// A write function, decoded from A5-A3 with the fetcher in A2-A0.
    fn write_function(&mut self, offset: u16, data: u8) {
        let index = usize::from(offset & 0x07);
        match (offset >> 3) & 0x07 {
            0 => {
                self.fetchers[index].top = data;
                self.fetchers[index].flag = false;
            }
            1 => self.fetchers[index].bottom = data,
            2 => {
                // A music voice restarts its phase: the write loads Top, not
                // the byte the CPU put on the bus.
                let low = match self.fetchers[index].music {
                    true => self.fetchers[index].top,
                    false => data,
                };
                self.fetchers[index].counter =
                    (self.fetchers[index].counter & 0x700) | u16::from(low);
                self.fetchers[index].evaluate_flag();
            }
            3 => {
                let fetcher = &mut self.fetchers[index];
                fetcher.counter = (u16::from(data & 0x07) << 8) | (fetcher.counter & 0xFF);
                if index == DRAW_LINE_FETCHER {
                    self.draw_line.enabled = data & 0x10 != 0;
                } else if index >= FIRST_MUSIC_FETCHER {
                    fetcher.music = data & 0x10 != 0;
                    fetcher.oscillator_clocked = data & 0x20 != 0;
                }
            }
            4 => self.draw_line.movamt = data >> 4,
            6 => self.rng = 0,
            _ => {}
        }
    }

    /// Writing DF4's counter low also seeds the draw-line latch.
    fn seed_draw_line(&mut self, offset: u16, data: u8) {
        if (offset >> 3) & 0x07 == 2 && usize::from(offset & 0x07) == DRAW_LINE_FETCHER {
            self.draw_line.latch = data;
        }
    }

    pub fn read(&mut self, address: u16, residue: u8) -> u8 {
        self.clock_rng();
        if !Dpc::is_register(address) {
            self.hotspot(address);
            return self.peek(address);
        }
        let offset = address & 0x7F;
        if offset & WRITE_FUNCTION == 0 {
            return self.read_function(offset);
        }
        // No R/W line: a read of a write window fires the write decoders while
        // nothing drives the bus, so the board latches the floating byte.
        self.seed_draw_line(offset, residue);
        self.write_function(offset, residue);
        residue
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        self.clock_rng();
        if !Dpc::is_register(address) {
            self.hotspot(address);
            return;
        }
        let offset = address & 0x7F;
        if offset & WRITE_FUNCTION == 0 {
            // A write to a read window still fires the read decoders, so the
            // fetcher steps; nothing takes the CPU's byte.
            self.read_function(offset);
            return;
        }
        self.seed_draw_line(offset, data);
        self.write_function(offset, data);
    }

    /// The banked program ROM, both banks in file order, for the debugger's
    /// bank-complete `rom` region. The display ROM is not CPU-addressable — it
    /// is reachable only through the fetchers — so it is not part of this view.
    pub(super) fn rom(&self) -> &[u8] {
        &self.program
    }

    /// The selected F8 program bank, for a state save. The display-ROM fetchers
    /// and RNG are the custom chip's transient run state, not bank selection.
    pub(super) fn bank_state(&self) -> Vec<u8> {
        vec![self.bank as u8]
    }

    pub(super) fn restore_bank_state(&mut self, bytes: &[u8]) {
        if let Some(&bank) = bytes.first() {
            self.bank = (bank as usize) % (self.program.len() / BANK_SIZE).max(1);
        }
    }

    /// A read-only view of the chip's internals for the debugger.
    pub(super) fn inspect(&self) -> DpcView {
        DpcView {
            fetchers: std::array::from_fn(|i| {
                let f = &self.fetchers[i];
                DpcFetcherView {
                    counter: f.counter & 0x7FF,
                    top: f.top,
                    bottom: f.bottom,
                    flag: f.flag,
                    music: f.music,
                    oscillator: f.oscillator_clocked,
                }
            }),
            rng: self.rng,
            bank: self.bank,
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.program[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four console clocks per oscillation, so a test need not count thousands.
    fn voice_on(oscillator: bool) -> Dpc {
        let mut dpc = Dpc::new(&vec![0; 0x2900], OSCILLATOR_HZ * 4.0);
        dpc.write_access(0x1045, 1); // DF5 Top = 1, so the voice's period is 2
        dpc.write_access(0x105D, if oscillator { 0x30 } else { 0x10 });
        dpc.write_access(0x1055, 0); // counter low: in music mode this loads Top
        dpc
    }

    /// The suite cannot reach this: its ROMs fix the clock select to the read
    /// strobe, and the oscillator's pitch is ruled out of scope as a per-cart
    /// RC value. So this is the only check that a voice handed to the
    /// oscillator advances — a game whose music nothing reads depends on it.
    #[test]
    fn an_oscillator_clocked_voice_advances_unread() {
        let mut dpc = voice_on(true);
        // The counter loaded Top, so the comparator has set the flag and the
        // voice is sounding.
        assert_eq!(dpc.read(0xF006, 0), AMPLITUDE[1]);
        for _ in 0..4 {
            dpc.tick();
        }
        // One oscillation later it has counted down onto Bottom and gone quiet,
        // with nothing having read the fetcher.
        assert_eq!(dpc.read(0xF006, 0), AMPLITUDE[0]);
    }

    /// The other half of the select: a voice clocked from its read strobe must
    /// ignore the oscillator, which free-runs regardless.
    #[test]
    fn a_strobe_clocked_voice_ignores_the_oscillator() {
        let mut dpc = voice_on(false);
        assert_eq!(dpc.read(0xF006, 0), AMPLITUDE[1]);
        for _ in 0..64 {
            dpc.tick();
        }
        assert_eq!(dpc.read(0xF006, 0), AMPLITUDE[1]);
    }
}
