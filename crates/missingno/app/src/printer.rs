//! Game Boy Printer emulation, attached to the link port whenever no other
//! link is in use. It stays inert until a game speaks the printer protocol
//! (an idle printer answers like an unplugged cable), so it can always be
//! connected. Completed prints are handed to the app to log against the
//! current play session.

use std::sync::mpsc::Sender;

use missingno_gb::serial_transfer::SerialLink;

/// A finished print handed up to the app: 160 wide, self-sized height,
/// one grayscale byte per pixel (row-major).
pub struct CompletedPrint {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Where a [`GbPrinter`] sends finished prints for the play log to record.
pub type PrintSink = Sender<CompletedPrint>;

const MAGIC: [u8; 2] = [0x88, 0x33];

const CMD_INIT: u8 = 0x01;
const CMD_PRINT: u8 = 0x02;
const CMD_DATA: u8 = 0x04;
#[cfg(test)]
const CMD_STATUS: u8 = 0x0f;

const STATUS_CHECKSUM_ERROR: u8 = 0x01;
const STATUS_PRINTING: u8 = 0x02;
const STATUS_BUFFER_FULL: u8 = 0x04;
const STATUS_DATA_READY: u8 = 0x08;

/// The real printer's receive buffer.
const BUFFER_CAPACITY: usize = 0x2000;
/// Tiles per 160px-wide row.
const TILES_PER_ROW: usize = 20;
const TILE_BYTES: usize = 16;

/// M-cycles of line silence before the packet parser resets — the hardware
/// gives up after ~100 ms.
const IDLE_RESET_MCYCLES: u32 = 105_000;

/// How long a print reports "printing" before completing, in M-cycles.
/// The real device takes seconds; games only need the phase to exist.
const PRINT_MCYCLES: u32 = 500_000;

enum Field {
    Magic0,
    Magic1,
    Command,
    Compression,
    LengthLow,
    LengthHigh,
    Payload,
    ChecksumLow,
    ChecksumHigh,
    Alive,
    Status,
}

pub struct GbPrinter {
    sink: PrintSink,

    field: Field,
    command: u8,
    compressed: bool,
    remaining: u16,
    payload: Vec<u8>,
    checksum: u16,
    received_checksum: u16,

    buffer: Vec<u8>,
    status: u8,
    printing_left: u32,

    bit_count: u8,
    in_shift: u8,
    out_shift: u8,
    idle_mcycles: u32,
}

impl GbPrinter {
    pub fn new(sink: PrintSink) -> Self {
        Self {
            sink,
            field: Field::Magic0,
            command: 0,
            compressed: false,
            remaining: 0,
            payload: Vec::new(),
            checksum: 0,
            received_checksum: 0,
            buffer: Vec::new(),
            status: 0,
            printing_left: 0,
            bit_count: 0,
            in_shift: 0,
            out_shift: 0,
            idle_mcycles: 0,
        }
    }

    /// Process one received byte and return the byte to present during the
    /// next exchange.
    fn push_byte(&mut self, byte: u8) -> u8 {
        match self.field {
            Field::Magic0 => {
                if byte == MAGIC[0] {
                    self.field = Field::Magic1;
                }
                0
            }
            Field::Magic1 => {
                self.field = if byte == MAGIC[1] {
                    Field::Command
                } else {
                    Field::Magic0
                };
                0
            }
            Field::Command => {
                self.command = byte;
                self.checksum = byte as u16;
                self.field = Field::Compression;
                0
            }
            Field::Compression => {
                self.compressed = byte & 1 != 0;
                self.checksum = self.checksum.wrapping_add(byte as u16);
                self.field = Field::LengthLow;
                0
            }
            Field::LengthLow => {
                self.remaining = byte as u16;
                self.checksum = self.checksum.wrapping_add(byte as u16);
                self.field = Field::LengthHigh;
                0
            }
            Field::LengthHigh => {
                self.remaining |= (byte as u16) << 8;
                self.checksum = self.checksum.wrapping_add(byte as u16);
                self.payload.clear();
                self.field = if self.remaining == 0 {
                    Field::ChecksumLow
                } else {
                    Field::Payload
                };
                0
            }
            Field::Payload => {
                self.payload.push(byte);
                self.checksum = self.checksum.wrapping_add(byte as u16);
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.field = Field::ChecksumLow;
                }
                0
            }
            Field::ChecksumLow => {
                self.received_checksum = byte as u16;
                self.field = Field::ChecksumHigh;
                0
            }
            Field::ChecksumHigh => {
                self.received_checksum |= (byte as u16) << 8;
                self.field = Field::Alive;
                0x81
            }
            Field::Alive => {
                // The keepalive slot already answered 0x81; run the command
                // so the next slot carries its status.
                self.field = Field::Status;
                if self.received_checksum != self.checksum {
                    self.status |= STATUS_CHECKSUM_ERROR;
                } else {
                    self.execute_command();
                }
                self.status
            }
            Field::Status => {
                self.field = Field::Magic0;
                self.status &= !STATUS_CHECKSUM_ERROR;
                0
            }
        }
    }

    fn execute_command(&mut self) {
        match self.command {
            CMD_INIT => {
                self.buffer.clear();
                self.status = 0;
                self.printing_left = 0;
            }
            CMD_DATA => {
                let data = if self.compressed {
                    decompress(&self.payload)
                } else {
                    self.payload.clone()
                };
                if self.buffer.len() + data.len() > BUFFER_CAPACITY {
                    self.status |= STATUS_BUFFER_FULL;
                } else {
                    self.buffer.extend_from_slice(&data);
                }
                if !self.buffer.is_empty() {
                    self.status |= STATUS_DATA_READY;
                }
            }
            CMD_PRINT => {
                let palette = self.payload.get(2).copied().unwrap_or(0xe4);
                self.print(palette);
                self.buffer.clear();
                self.status &= !(STATUS_DATA_READY | STATUS_BUFFER_FULL);
                self.status |= STATUS_PRINTING;
                self.printing_left = PRINT_MCYCLES;
            }
            _ => {}
        }
    }

    fn print(&mut self, palette: u8) {
        let Some(pixels) = render(&self.buffer, palette) else {
            return;
        };
        let width = TILES_PER_ROW as u32 * 8;
        let height = pixels.len() as u32 / width;
        let _ = self.sink.send(CompletedPrint {
            width,
            height,
            pixels,
        });
    }
}

/// Decode a print buffer of 2bpp tiles through a BGP-style palette into
/// row-major grayscale pixels; `None` until at least one full tile row exists.
fn render(buffer: &[u8], palette: u8) -> Option<Vec<u8>> {
    let tile_rows = buffer.len() / (TILES_PER_ROW * TILE_BYTES);
    if tile_rows == 0 {
        return None;
    }
    let width = TILES_PER_ROW * 8;
    let mut pixels = vec![0u8; width * tile_rows * 8];
    for (tile, bytes) in buffer.chunks_exact(TILE_BYTES).enumerate() {
        let tile_x = tile % TILES_PER_ROW;
        let tile_y = tile / TILES_PER_ROW;
        if tile_y >= tile_rows {
            break;
        }
        for row in 0..8 {
            let low = bytes[row * 2];
            let high = bytes[row * 2 + 1];
            for column in 0..8 {
                let bit = 7 - column;
                let color = ((high >> bit) & 1) << 1 | ((low >> bit) & 1);
                let shade = (palette >> (color * 2)) & 3;
                let y = tile_y * 8 + row;
                let x = tile_x * 8 + column;
                pixels[y * width + x] = [255, 170, 85, 0][shade as usize];
            }
        }
    }
    Some(pixels)
}

/// Printer RLE: control bit 7 set = repeat the next byte (low 7 bits + 2)
/// times; clear = copy (low 7 bits + 1) literal bytes.
fn decompress(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut rest = data;
    while let [control, tail @ ..] = rest {
        if control & 0x80 != 0 {
            let [value, tail @ ..] = tail else { break };
            output.extend(std::iter::repeat_n(*value, (control & 0x7f) as usize + 2));
            rest = tail;
        } else {
            let length = (*control as usize + 1).min(tail.len());
            output.extend_from_slice(&tail[..length]);
            rest = &tail[length..];
        }
    }
    output
}

impl SerialLink for GbPrinter {
    fn exchange_bit(&mut self, out_bit: bool) -> bool {
        self.idle_mcycles = 0;
        let reply = self.out_shift & 0x80 != 0;
        self.out_shift <<= 1;
        self.in_shift = (self.in_shift << 1) | out_bit as u8;
        self.bit_count += 1;
        if self.bit_count == 8 {
            self.bit_count = 0;
            self.out_shift = self.push_byte(self.in_shift);
        }
        reply
    }

    /// The printer never drives the clock.
    fn clock(&mut self) -> bool {
        false
    }

    fn tick(&mut self) {
        if self.printing_left > 0 {
            self.printing_left -= 1;
            if self.printing_left == 0 {
                self.status &= !STATUS_PRINTING;
            }
        }
        // A quiet line resets a half-parsed packet, like the hardware's
        // ~100 ms timeout.
        if !matches!(self.field, Field::Magic0) {
            self.idle_mcycles += 1;
            if self.idle_mcycles > IDLE_RESET_MCYCLES {
                self.field = Field::Magic0;
                self.bit_count = 0;
                self.idle_mcycles = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A printer whose finished prints are discarded; these tests exercise the
    /// packet protocol, not print output.
    fn test_printer() -> GbPrinter {
        GbPrinter::new(std::sync::mpsc::channel().0)
    }

    fn send_packet(printer: &mut GbPrinter, command: u8, payload: &[u8]) -> (u8, u8) {
        let mut checksum = command as u16;
        checksum = checksum.wrapping_add(0); // compression flag
        checksum = checksum.wrapping_add((payload.len() as u16) & 0xff);
        checksum = checksum.wrapping_add((payload.len() as u16) >> 8);
        for &byte in payload {
            checksum = checksum.wrapping_add(byte as u16);
        }

        printer.push_byte(0x88);
        printer.push_byte(0x33);
        printer.push_byte(command);
        printer.push_byte(0x00);
        printer.push_byte((payload.len() & 0xff) as u8);
        printer.push_byte((payload.len() >> 8) as u8);
        for &byte in payload {
            printer.push_byte(byte);
        }
        printer.push_byte((checksum & 0xff) as u8);
        let alive = printer.push_byte((checksum >> 8) as u8);
        let status = printer.push_byte(0x00);
        printer.push_byte(0x00);
        (alive, status)
    }

    #[test]
    fn packet_flow_reports_alive_data_and_printing() {
        let mut printer = test_printer();
        let (alive, status) = send_packet(&mut printer, CMD_INIT, &[]);
        assert_eq!(alive, 0x81);
        assert_eq!(status, 0);

        let band = vec![0x55; TILES_PER_ROW * TILE_BYTES];
        let (_, status) = send_packet(&mut printer, CMD_DATA, &band);
        assert_eq!(status & STATUS_DATA_READY, STATUS_DATA_READY);

        let (_, status) = send_packet(&mut printer, CMD_STATUS, &[]);
        assert_eq!(status & STATUS_DATA_READY, STATUS_DATA_READY);
    }

    #[test]
    fn print_command_emits_a_completed_print() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut printer = GbPrinter::new(tx);
        send_packet(&mut printer, CMD_INIT, &[]);
        // One full tile row of data, then a print with an identity palette
        // (payload byte 2 selects it).
        let band = vec![0x55; TILES_PER_ROW * TILE_BYTES];
        send_packet(&mut printer, CMD_DATA, &band);
        send_packet(&mut printer, CMD_PRINT, &[0x01, 0x00, 0xe4, 0x40]);

        let print = rx.try_recv().expect("a print was emitted");
        assert_eq!(print.width, 160);
        assert_eq!(print.height, 8);
        assert_eq!(print.pixels.len(), 160 * 8);
    }

    #[test]
    fn bad_checksum_flags_the_error_once() {
        let mut printer = test_printer();
        printer.push_byte(0x88);
        printer.push_byte(0x33);
        printer.push_byte(CMD_STATUS);
        printer.push_byte(0x00);
        printer.push_byte(0x00);
        printer.push_byte(0x00);
        printer.push_byte(0xff); // wrong checksum
        let alive = printer.push_byte(0xff);
        let status = printer.push_byte(0x00);
        printer.push_byte(0x00);
        assert_eq!(alive, 0x81);
        assert_eq!(status & STATUS_CHECKSUM_ERROR, STATUS_CHECKSUM_ERROR);

        let (_, status) = send_packet(&mut printer, CMD_STATUS, &[]);
        assert_eq!(status & STATUS_CHECKSUM_ERROR, 0);
    }

    #[test]
    fn rle_decompresses_runs_and_literals() {
        // Repeat 0xAA ×4 (control 0x82), then 3 literals (control 0x02).
        let decoded = decompress(&[0x82, 0xaa, 0x02, 1, 2, 3]);
        assert_eq!(decoded, [0xaa, 0xaa, 0xaa, 0xaa, 1, 2, 3]);
    }

    #[test]
    fn render_maps_2bpp_through_the_palette() {
        // One tile whose first row is color 3 across (low+high bits all set),
        // remaining rows color 0; pad to a full 20-tile row.
        let mut buffer = vec![0u8; TILES_PER_ROW * TILE_BYTES];
        buffer[0] = 0xff;
        buffer[1] = 0xff;
        let pixels = render(&buffer, 0xe4).unwrap();
        assert_eq!(pixels.len(), 160 * 8);
        assert_eq!(pixels[0], 0); // color 3 → black under 0xE4
        assert_eq!(pixels[160], 255); // color 0 → white
        assert_eq!(pixels[8], 255); // next tile, untouched
    }

    #[test]
    fn bits_assemble_into_bytes_msb_first() {
        let mut printer = test_printer();
        // Shift in 0x88 then 0x33; the printer should now expect a command.
        for byte in [0x88u8, 0x33] {
            for bit in (0..8).rev() {
                printer.exchange_bit(byte & (1 << bit) != 0);
            }
        }
        assert!(matches!(printer.field, Field::Command));
    }
}
