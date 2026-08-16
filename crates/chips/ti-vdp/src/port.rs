//! The CPU port and the VRAM access engine behind it: the two control-port
//! bytes, the read-ahead buffer, and the memory-cycle schedule a transfer
//! waits for.

use crate::Vdp;
use crate::registers::r1;
use crate::standard::ACTIVE_LINES;
use crate::vram::VramAddress;

/// One DRAM memory cycle is two dots = four XTAL periods; 171 per line.
pub(crate) const CYCLES_PER_LINE: usize = 171;
/// The eleven runs of consecutive CPU-access memory cycles on an active
/// non-text line, as run-start positions — starts spaced
/// 16,16,16,16,15,16,15,13,16,16,16 cycles (sum 171), the lattice both
/// silicon maps trace out. The rotation against hsync is unmeasured, so
/// the origin is a free convention.
const RUN_START_CYCLES: [usize; 11] = [0, 16, 32, 48, 64, 79, 95, 110, 123, 139, 155];
/// Run lengths, shortest of the map-equivalent family (19 CPU cycles per
/// line, the documented CPU-cycle budget).
const RUN_LENGTH_CYCLES: [usize; 11] = [1, 1, 1, 1, 1, 2, 5, 4, 1, 1, 1];
/// The servicing cycle samples the transfer register this long after its
/// own start...
const TRANSFER_LOCK_XTALS: u64 = 17;
/// ...and the request flag clears this long after, so a request landing
/// in the gap both supplies the in-flight data and queues itself.
const FLAG_RELEASE_XTALS: u64 = 15;
/// A port write needs this long to settle; a lock sampling sooner sees
/// the register mid-transition and each bit resolves as old AND new.
const TRANSFER_SETTLE_XTALS: u64 = 2;
/// The fetch schedule wakes this many lines before display line 0; the
/// measured turn-on seam sits ~2.6 lines before, the line boundary being
/// the model's quantum.
const SCHEDULE_WARM_UP_LINES: u16 = 3;

/// Whether each memory cycle of a rendering line is a CPU-access cycle.
const ACCESS_CYCLES: [bool; CYCLES_PER_LINE] = {
    let mut map = [false; CYCLES_PER_LINE];
    let mut run = 0;
    while run < RUN_START_CYCLES.len() {
        let mut offset = 0;
        while offset < RUN_LENGTH_CYCLES[run] {
            map[RUN_START_CYCLES[run] + offset] = true;
            offset += 1;
        }
        run += 1;
    }
    map
};

/// Direction and data of a CPU port transfer.
#[derive(Clone, Copy)]
pub(crate) enum PortTransfer {
    Write(u8),
    Refill,
}

/// The command bits 7-6 of the control port's second byte carry.
enum ControlCommand {
    ArmRead,
    ArmWrite,
    /// Bits 2-0 name the register; the value is the first byte.
    WriteRegister(usize),
}

impl ControlCommand {
    fn decode(value: u8) -> Self {
        match value & 0xC0 {
            0x00 => ControlCommand::ArmRead,
            0x40 => ControlCommand::ArmWrite,
            _ => ControlCommand::WriteRegister((value & 0x07) as usize),
        }
    }
}

/// The access a CPU-access cycle has claimed: the address latched at
/// claim, and the cycle's lock and release instants.
pub(crate) struct InFlightAccess {
    address: VramAddress,
    lock_at: u64,
    release_at: u64,
}

/// The port's own state: the pointer, the control-port latch, the
/// read-ahead buffer, and the transfer waiting on a memory cycle.
pub(crate) struct Port {
    /// Auto-incrementing VRAM pointer.
    pub(crate) address: VramAddress,
    /// Set between the first and second control-port bytes.
    pub(crate) awaiting_second_byte: bool,
    pub(crate) read_buffer: u8,
    /// Re-latched by every CPU request; the servicing cycle samples it at
    /// its lock instant, so a newer request steals an in-flight cycle.
    transfer: PortTransfer,
    /// The transfer being replaced, and when: a lock sampling within the
    /// settle window merges the two.
    prior_transfer: PortTransfer,
    transfer_written_at: u64,
    /// Latched by the request that raised the flag; replacements leave it
    /// holding — a superseded access keeps the first address.
    pending_address: VramAddress,
    pending_flag: bool,
    in_flight: Option<InFlightAccess>,
}

impl Port {
    pub(crate) const POWER_ON: Self = Port {
        address: VramAddress::ZERO,
        awaiting_second_byte: false,
        read_buffer: 0,
        transfer: PortTransfer::Refill,
        prior_transfer: PortTransfer::Refill,
        transfer_written_at: 0,
        pending_address: VramAddress::ZERO,
        pending_flag: false,
        in_flight: None,
    };
}

impl Vdp {
    pub fn address(&self) -> u16 {
        self.port.address.value()
    }

    pub fn read_buffer(&self) -> u8 {
        self.port.read_buffer
    }

    /// Whether the control port holds a first byte awaiting its second.
    pub fn awaiting_second_byte(&self) -> bool {
        self.port.awaiting_second_byte
    }

    pub fn write_control(&mut self, value: u8) {
        if !self.port.awaiting_second_byte {
            // The first byte lands in the pointer immediately.
            self.port.address = self.port.address.with_low(value);
            self.port.awaiting_second_byte = true;
            return;
        }
        self.port.awaiting_second_byte = false;
        // The second byte always lands in the pointer's high bits — a
        // register write leaves the pointer pointing at (byte2 & $3F)
        // << 8 | byte1, which is what "destroys" a pending address.
        self.port.address = self.port.address.with_high(value);
        match ControlCommand::decode(value) {
            ControlCommand::ArmRead => self.fill_read_buffer(),
            ControlCommand::ArmWrite => {}
            ControlCommand::WriteRegister(register) => {
                self.registers[register] = self.port.address.low();
            }
        }
    }

    pub fn write_data(&mut self, value: u8) {
        self.port.awaiting_second_byte = false;
        // A write loads the read-ahead buffer with the written value.
        self.port.read_buffer = value;
        let address = self.port.address;
        self.request(address, PortTransfer::Write(value));
        self.increment_address();
    }

    pub fn read_data(&mut self) -> u8 {
        self.port.awaiting_second_byte = false;
        let value = self.port.read_buffer;
        let address = self.port.address;
        self.request(address, PortTransfer::Refill);
        self.increment_address();
        value
    }

    fn fill_read_buffer(&mut self) {
        let address = self.port.address;
        self.request(address, PortTransfer::Refill);
        self.increment_address();
    }

    fn increment_address(&mut self) {
        self.port.address = self.port.address.incremented();
    }

    /// Every CPU access re-latches direction and data; the address only
    /// latches with the request that raises the flag, so a superseded
    /// access carries the first address with the last value.
    fn request(&mut self, address: VramAddress, transfer: PortTransfer) {
        self.port.prior_transfer = self.port.transfer;
        self.port.transfer = transfer;
        self.port.transfer_written_at = self.xtal_total;
        if !self.port.pending_flag {
            self.port.pending_address = address;
            self.port.pending_flag = true;
        }
    }

    /// The claim → release → lock events of the CPU port at this instant.
    pub(crate) fn service_port(&mut self) {
        if self.xtal_in_line.is_multiple_of(4)
            && self.port.pending_flag
            && self.port.in_flight.is_none()
            && self.cycle_accessible((self.xtal_in_line / 4) as usize)
        {
            self.port.in_flight = Some(InFlightAccess {
                address: self.port.pending_address,
                lock_at: self.xtal_total + TRANSFER_LOCK_XTALS,
                release_at: self.xtal_total + FLAG_RELEASE_XTALS,
            });
        }
        if let Some(access) = &self.port.in_flight {
            if self.xtal_total == access.release_at {
                self.port.pending_flag = false;
            }
            if self.xtal_total == access.lock_at {
                let address = access.address;
                self.port.in_flight = None;
                self.complete(address);
            }
        }
    }

    /// Rendering lines and the pre-display warm-up tail confine CPU access
    /// to the run schedule; everywhere else every memory cycle is claimable.
    fn cycle_accessible(&self, cycle: usize) -> bool {
        let scheduled_line = self.line < ACTIVE_LINES
            || self.line >= self.standard.lines_per_frame() - SCHEDULE_WARM_UP_LINES;
        let rendering = self.display_enabled() && self.registers[1] & r1::M1 == 0 && scheduled_line;
        !rendering || ACCESS_CYCLES[cycle]
    }

    /// The transfer as the lock samples it: settled, the latest value;
    /// mid-transition, writes merge bitwise old AND new. A direction bit
    /// mid-flip is unmeasured — the latest transfer wins.
    fn locked_transfer(&self) -> PortTransfer {
        if self.xtal_total - self.port.transfer_written_at >= TRANSFER_SETTLE_XTALS {
            return self.port.transfer;
        }
        match (self.port.prior_transfer, self.port.transfer) {
            (PortTransfer::Write(old), PortTransfer::Write(new)) => PortTransfer::Write(old & new),
            (_, latest) => latest,
        }
    }

    fn complete(&mut self, address: VramAddress) {
        match self.locked_transfer() {
            PortTransfer::Write(value) => self.store(address, value),
            PortTransfer::Refill => self.port.read_buffer = self.fetch(address),
        }
    }
}
