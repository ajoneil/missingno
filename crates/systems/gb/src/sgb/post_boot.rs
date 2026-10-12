//! The state the SGB2 boot ROM hands off at PC=$0100. It sends the header to
//! the SNES as six packets, a zero bit one M-cycle slower than a set bit, and
//! after each polls LY in 8 M-cycle passes, so the zero-bit count mod 8 lasts.

use crate::audio::{ApuSpec, Audio, Register};
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::cpu::flags::Flags;
use crate::joypad::{Buttons, Joypad};
use crate::memory::{ExternalBus, HighRam};
use crate::ppu::Ppu;
use crate::ppu::model::PpuModel;
use crate::serial_transfer::Serial;
use crate::timers::Timers;

/// The internal counter at the earliest handoff.
const EARLIEST_COUNTER: u16 = 0xF611;
/// The PPU's horizontal counter on line 153 at the earliest handoff.
const EARLIEST_LX: u8 = 34;
/// The zero-bit count, mod 8, whose packets reach the earliest handoff.
const EARLIEST_ZERO_BITS: u32 = 4;

const PACKETS: usize = 6;
const PACKET_LEN: usize = 16;
const FIRST_COMMAND: u8 = 0xF1;
/// The header bytes each full packet carries; the last carries the rest.
const HEADER_PER_PACKET: usize = 14;
const HEADER_START: usize = 0x0104;
const HEADER_END: usize = 0x0150;
/// Where the boot ROM builds the packets in work RAM ($C000).
const PACKET_BUFFER: usize = 0x0000;

/// The packets as the boot ROM builds them: a command, the sum of the data
/// bytes, then the header from $0104 on, the last packet padded with zeros.
fn header_packets(cartridge: &Cartridge) -> [[u8; PACKET_LEN]; PACKETS] {
    let header = &cartridge.rom()[HEADER_START..HEADER_END];
    std::array::from_fn(|index| {
        let start = index * HEADER_PER_PACKET;
        let data = &header[start..(start + HEADER_PER_PACKET).min(header.len())];
        let mut packet = [0; PACKET_LEN];
        packet[0] = FIRST_COMMAND + 2 * index as u8;
        packet[1] = data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        packet[2..2 + data.len()].copy_from_slice(data);
        packet
    })
}

/// M-cycles past the earliest handoff, 0 to 7.
pub fn handoff_delay(cartridge: &Cartridge) -> u8 {
    let zero_bits: u32 = header_packets(cartridge)
        .iter()
        .flatten()
        .map(|byte| byte.count_zeros())
        .sum();
    ((zero_bits + 8 - EARLIEST_ZERO_BITS) % 8) as u8
}

pub(crate) fn cpu() -> Cpu {
    Cpu::post_boot_with(0xFF, 0x00, 0x14, 0x00, 0x00, 0xC0, 0x60, Flags::empty())
}

pub(crate) fn timers(cartridge: &Cartridge) -> Timers {
    Timers::post_boot_with_counter(EARLIEST_COUNTER + u16::from(handoff_delay(cartridge)))
}

/// The handoff falls on line 153 while LY reads 0, as on the DMG, but earlier
/// in the line.
pub(crate) fn ppu<P: PpuModel>(cartridge: &Cartridge) -> Ppu<P> {
    Ppu::post_boot_at_x(EARLIEST_LX + handoff_delay(cartridge))
}

/// The boot ROM powers the APU and sets up CH1 without triggering it: the
/// SGB plays no boot chime. By the handoff the frame sequencer has reached
/// step 3, wherever in its 8 M-cycles the handoff lands, and CH1's envelope
/// timer, which runs while the channel is off, has counted down to 2.
pub(crate) fn audio<A: ApuSpec>(internal_counter: u16) -> Audio<A> {
    let mut audio = Audio::new();
    for (address, value) in [
        (0xFF26, 0x80),
        (0xFF11, 0x80),
        (0xFF12, 0xF3),
        (0xFF25, 0xF3),
        (0xFF24, 0x77),
        (0xFF13, 0xC1),
        (0xFF14, 0x07),
    ] {
        audio.write_register(Register::map(address), value, internal_counter, false);
    }
    audio.frame_sequencer_step = 3;
    audio.channels.ch1.envelope.timer = 2;
    audio.channels.ch3.ch3_2mhz = true;
    audio.prev_div_apu_bit = internal_counter & crate::audio::DIV_APU_BIT != 0;
    audio
}

/// The boot ROM's last JOYP write deselects both key-matrix lines.
pub(crate) fn joypad() -> Joypad {
    Joypad {
        read_buttons: false,
        read_dpad: false,
        pressed: Buttons::empty(),
    }
}

/// The serial clock toggles on each fall of counter bit 5, so at the handoff
/// it is high exactly when bit 6 is clear, as at the DMG handoff.
pub(crate) fn serial(internal_counter: u16) -> Serial {
    let mut serial = Serial::new();
    serial.registers.serial_clock = internal_counter & 0x40 == 0;
    serial.registers.previous_counter = internal_counter;
    serial
}

/// The packets stay in work RAM. The stack keeps the last return address,
/// $00B2, under the last register pair the logo routine pushed: B=$01 and
/// C=$39, from the Nintendo logo's last byte (the SNES refuses any other logo).
pub(crate) fn memory(external: &mut ExternalBus, high_ram: &mut HighRam) {
    let packets = header_packets(&external.cartridge);
    for (offset, byte) in packets.iter().flatten().enumerate() {
        external.work_ram[PACKET_BUFFER + offset] = *byte;
    }
    for (offset, byte) in [(0x7A, 0x39), (0x7B, 0x01), (0x7C, 0xB2), (0x7D, 0x00)] {
        high_ram.write(offset, byte);
    }
}
