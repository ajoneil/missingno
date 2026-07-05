use std::io::{Read, Write};
use std::time::Duration;

use super::detect::{DEFAULT_BAUD, cleanup, enter_dmg_mode, query_firmware_info};
use super::format_size;
use super::protocol::{
    DMG_CART_READ, DMG_SET_BANK_CHANGE_CMD, FLASH_PROGRAM, OFW_PCB_VER, SET_FLASH_CMD, cart_write,
    cart_write_flash, read_byte, set_variable, write_cmd, write_cmd_ack,
};

// ── Flash detection ──────────────────────────────────────────────────

/// Probe for a flash chip and query its parameters via CFI.
///
/// First probes with the AMD/JEDEC ID command to detect a flash chip, then
/// queries the CFI (Common Flash Interface) table for size, buffer, and sector
/// layout. Returns None for standard ROM cartridges.
///
/// Safe on commercial cartridges: writes use the flash write pin (WR) which
/// has no effect on standard ROM chips.
pub(super) fn detect_flash(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Option<FlashInfo> {
    // Set the flash write-enable pin to WR (pin mode 1)
    set_variable(port, fw_ver, 1, 0x04, 1)?;

    // Read original ROM data at address 0
    let original = read_rom_bytes(port, fw_ver, 0, 8)?;

    // Send AMD flash ID command sequence
    cart_write_flash(
        port,
        &[(0x0AAA, 0x00AA), (0x0555, 0x0055), (0x0AAA, 0x0090)],
    )?;

    // Read back — if data changed, a flash chip responded
    let chip_id = read_rom_bytes(port, fw_ver, 0, 8)?;

    // Reset flash back to read mode
    let _ = cart_write_flash(port, &[(0x0000, 0x00F0)]);

    if original == chip_id {
        return None;
    }

    // Query CFI table
    let cfi = query_cfi(port, fw_ver);

    // Reset again after CFI query
    let _ = cart_write_flash(port, &[(0x0000, 0x00F0)]);

    match cfi {
        Some(info) => Some(FlashInfo { chip_id, ..info }),
        None => None,
    }
}

/// Query the CFI (Common Flash Interface) table from a flash chip.
///
/// Sends the CFI enter command (0x98), reads 0x400 bytes, parses the
/// standardised CFI structure for device size, write buffer, and sector layout.
fn query_cfi(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Option<FlashInfo> {
    // Enter CFI mode: write 0x98 to address 0x00AA
    cart_write_flash(port, &[(0x00AA, 0x0098)])?;

    // Read CFI table
    let cfi = read_rom_bytes(port, fw_ver, 0, 0x400)?;

    // Reset back to read mode
    let _ = cart_write_flash(port, &[(0x0000, 0x00F0)]);

    // Check for "QRY" magic at 16-bit offsets (0x20/0x22/0x24) or 8-bit (0x10/0x11/0x12)
    let is_16bit = cfi.len() > 0x24 && cfi[0x20] == b'Q' && cfi[0x22] == b'R' && cfi[0x24] == b'Y';
    let is_8bit = cfi.len() > 0x12 && cfi[0x10] == b'Q' && cfi[0x11] == b'R' && cfi[0x12] == b'Y';

    if !is_16bit && !is_8bit {
        return None;
    }

    let is_8bit = is_8bit && !is_16bit;

    // For 8-bit mode, expand to 16-bit layout (double each byte)
    let cfi = if is_8bit {
        cfi.iter().flat_map(|&b| [b, b]).collect::<Vec<u8>>()
    } else {
        cfi
    };

    // Parse CFI fields (all at 16-bit offsets, so multiply by 2)
    if cfi.len() < 0x60 {
        return None;
    }

    // Device size: 2^N bytes
    let device_size = 1u32 << cfi[0x4E];

    // Write buffer size
    let buffer_raw = (cfi[0x56] as u32) << 8 | cfi[0x54] as u32;
    let buffer_size = if buffer_raw > 1 {
        1u32 << buffer_raw
    } else {
        0
    };

    // Erase capabilities
    let sector_erase = cfi[0x42] > 0 && cfi[0x42] < 0xFF;
    let chip_erase = cfi[0x44] > 0 && cfi[0x44] < 0xFF;

    // Sector regions
    let num_regions = cfi[0x58] as usize;
    let mut sectors = Vec::new();
    for i in 0..num_regions.min(4) {
        let offset = 0x5A + i * 8;
        if offset + 7 >= cfi.len() {
            break;
        }
        let count = ((cfi[offset + 2] as u32) << 8 | cfi[offset] as u32) + 1;
        let size = ((cfi[offset + 6] as u32) << 8 | cfi[offset + 4] as u32) * 256;
        sectors.push((size, count));
    }

    Some(FlashInfo {
        chip_id: Vec::new(), // Filled in by caller
        size: device_size,
        buffer_size,
        chip_erase,
        sector_erase,
        sectors,
    })
}

/// Read a small number of bytes from ROM at a given address.
fn read_rom_bytes(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
    address: u32,
    count: u16,
) -> Option<Vec<u8>> {
    set_variable(port, fw_ver, 2, 0x00, count as u32)?; // TRANSFER_SIZE
    set_variable(port, fw_ver, 4, 0x00, address)?; // ADDRESS
    set_variable(port, fw_ver, 1, 0x01, 1)?; // DMG_ACCESS_MODE = ROM_READ
    write_cmd(port, &[DMG_CART_READ])?;
    let mut buf = vec![0u8; count as usize];
    port.read_exact(&mut buf).ok()?;
    Some(buf)
}

// ── ROM flashing ─────────────────────────────────────────────────────

/// Progress update during a flash operation.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlashProgress {
    pub phase: FlashPhase,
    pub bytes_done: usize,
    pub bytes_total: usize,
}

#[derive(Debug, Clone)]
pub enum FlashPhase {
    Erasing,
    Writing,
}

/// Flash a ROM to an inserted flash cartridge.
///
/// Erases the chip, then writes the ROM data. The cartridge must have been
/// previously detected with a valid `FlashInfo` (CFI query succeeded).
///
/// Designed to be called from a background thread via `smol::unblock`.
pub fn flash_rom(
    port_name: &str,
    flash: &FlashInfo,
    rom_data: &[u8],
    progress: &mut dyn FnMut(FlashProgress),
) -> Result<(), String> {
    if rom_data.is_empty() {
        return Err("ROM data is empty".to_string());
    }
    if rom_data.len() as u32 > flash.size {
        return Err(format!(
            "ROM ({}) is larger than flash chip ({})",
            format_size(rom_data.len() as u32),
            format_size(flash.size),
        ));
    }

    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(Duration::from_millis(2000))
        .open()
        .map_err(|e| format!("Failed to open port: {e}"))?;

    port.clear(serialport::ClearBuffer::All).ok();

    // Query firmware
    write_cmd(&mut port, &[OFW_PCB_VER]).ok_or("Failed to query PCB version")?;
    let _pcb = read_byte(&mut port).ok_or("No PCB version response")?;
    let (fw_ver, _) = query_firmware_info(&mut port).ok_or("Failed to query firmware")?;
    if fw_ver < 12 {
        return Err(format!("Firmware v{fw_ver} too old, need v12+"));
    }

    enter_dmg_mode(&mut port, fw_ver).ok_or("DMG mode setup failed")?;

    let write_method = flash.write_method();
    let we_pin: u8 = 0x01; // WR

    // Configure flash engine via SET_FLASH_CMD (0xA7)
    configure_flash_engine(&mut port, fw_ver, write_method, we_pin)?;

    // Configure auto bank switching (MBC5-style: bank number written to 0x2100)
    configure_bank_switching(&mut port)?;

    // Flash commands go to bank 0 (fixed region), not bank 1
    set_variable(&mut port, fw_ver, 1, 0x06, 0) // FLASH_COMMANDS_BANK_1 = 0
        .ok_or("Set FLASH_COMMANDS_BANK_1 failed")?;
    set_variable(&mut port, fw_ver, 1, 0x05, 0) // FLASH_PULSE_RESET = 0
        .ok_or("Set FLASH_PULSE_RESET failed")?;
    set_variable(&mut port, fw_ver, 1, 0x0A, 0) // FLASH_DOUBLE_DIE = 0
        .ok_or("Set FLASH_DOUBLE_DIE failed")?;

    // Set status register polling for AMD: wait for bit 7 (DQ7) = 1
    set_variable(&mut port, fw_ver, 2, 0x05, 0x0080) // STATUS_REGISTER_MASK
        .ok_or("Set STATUS_REGISTER_MASK failed")?;
    set_variable(&mut port, fw_ver, 2, 0x06, 0x0080) // STATUS_REGISTER_VALUE
        .ok_or("Set STATUS_REGISTER_VALUE failed")?;
    set_variable(&mut port, fw_ver, 1, 0x04, we_pin as u32) // FLASH_WE_PIN
        .ok_or("Set FLASH_WE_PIN failed")?;

    // ── Phase 1: Chip Erase ──
    progress(FlashProgress {
        phase: FlashPhase::Erasing,
        bytes_done: 0,
        bytes_total: rom_data.len(),
    });

    chip_erase_amd(&mut port, fw_ver)?;

    // ── Phase 2: Write ROM ──

    // Pad to 16K bank boundary
    let mut padded = rom_data.to_vec();
    if padded.len() % 0x4000 != 0 {
        padded.resize(padded.len() + (0x4000 - padded.len() % 0x4000), 0xFF);
    }

    let chunk_size = 0x100usize; // MAX_BUFFER_WRITE = 256 bytes
    let buffer_size = flash.buffer_size;
    let bank_size = 0x4000usize;
    let total = padded.len();
    let num_banks = total / bank_size;

    set_variable(&mut port, fw_ver, 2, 0x00, chunk_size as u32) // TRANSFER_SIZE
        .ok_or("Set TRANSFER_SIZE failed")?;
    set_variable(&mut port, fw_ver, 2, 0x01, buffer_size) // BUFFER_SIZE
        .ok_or("Set BUFFER_SIZE failed")?;

    // Write bank by bank. The firmware auto-increments within the bank window
    // (0x4000-0x7FFF) and uses DMG_ROM_BANK + DMG_SET_BANK_CHANGE_CMD for switching.
    for bank in 0..num_banks {
        let bank_offset = bank * bank_size;
        let bank_data = &padded[bank_offset..bank_offset + bank_size];

        // Skip entirely erased banks
        if bank_data.iter().all(|&b| b == 0xFF) {
            continue;
        }

        // Select bank via MBC register and set address to bank window
        cart_write(&mut port, fw_ver, 0x2100, bank as u8)
            .ok_or_else(|| format!("Bank select failed for bank {bank}"))?;
        set_variable(&mut port, fw_ver, 2, 0x02, bank as u32) // DMG_ROM_BANK
            .ok_or("Set DMG_ROM_BANK failed")?;
        set_variable(&mut port, fw_ver, 4, 0x00, 0x4000) // ADDRESS = bank window start
            .ok_or("Set ADDRESS failed")?;

        let mut last_ack: u8 = 0;

        for (ci, chunk) in bank_data.chunks(chunk_size).enumerate() {
            // Skip erased chunks
            if chunk.iter().all(|&b| b == 0xFF) {
                last_ack = 0;
                continue;
            }

            // Re-set address if streaming was broken
            if last_ack != 0x03 {
                let window_addr = 0x4000 + ci * chunk_size;
                set_variable(&mut port, fw_ver, 4, 0x00, window_addr as u32)
                    .ok_or("Set ADDRESS failed during write")?;
            }

            if last_ack != 0x03 {
                write_cmd(&mut port, &[FLASH_PROGRAM]).ok_or("FLASH_PROGRAM send failed")?;
            }

            port.write_all(chunk)
                .map_err(|e| format!("Write failed: {e}"))?;
            port.flush().map_err(|e| format!("Flush failed: {e}"))?;

            last_ack = read_byte(&mut port).ok_or("No ACK after flash program")?;
            if last_ack != 0x01 && last_ack != 0x03 {
                let abs_addr = bank_offset + ci * chunk_size;
                return Err(format!(
                    "Flash program failed at 0x{abs_addr:06X} (bank {bank}): ACK=0x{last_ack:02x}"
                ));
            }

            progress(FlashProgress {
                phase: FlashPhase::Writing,
                bytes_done: bank_offset + (ci + 1) * chunk_size,
                bytes_total: total,
            });
        }
    }

    // Reset flash to read mode
    let _ = cart_write_flash(&mut port, &[(0x0000, 0x00F0)]);

    // Disable auto bank switching
    let _ = write_cmd_ack(&mut port, &[DMG_SET_BANK_CHANGE_CMD, 0x00], fw_ver);

    cleanup(&mut port, fw_ver);

    Ok(())
}

/// Configure the flash engine via SET_FLASH_CMD (0xA7).
///
/// Sends: command_set (AMD=0x01), write method, WE pin, then 6 command slots.
/// For AMD single write: unlock1, unlock2, program command.
/// For AMD buffered write: unlock1, unlock2, buffer write setup.
fn configure_flash_engine(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
    write_method: u8,
    we_pin: u8,
) -> Result<(), String> {
    let mut buf = Vec::with_capacity(39);
    buf.push(SET_FLASH_CMD);
    buf.push(0x01); // AMD command set
    buf.push(write_method);
    buf.push(we_pin);

    // Command slots for AMD write
    let commands: &[(u32, u16)] = if write_method == 0x02 {
        // Buffered write
        &[
            (0x0AAA, 0x00AA), // unlock 1
            (0x0555, 0x0055), // unlock 2
            (0x0000, 0x0025), // SA + buffer write setup (SA filled by firmware)
            (0x0000, 0x0000), // SA + buffer size (BS filled by firmware)
            (0x0000, 0x0000), // PA + PD (filled by firmware)
            (0x0000, 0x0029), // SA + buffer write confirm
        ]
    } else {
        // Single/unbuffered write
        &[
            (0x0AAA, 0x00AA), // unlock 1
            (0x0555, 0x0055), // unlock 2
            (0x0AAA, 0x00A0), // program command
            (0x0000, 0x0000), // unused
            (0x0000, 0x0000), // unused
            (0x0000, 0x0000), // unused
        ]
    };

    for &(addr, val) in commands {
        buf.extend_from_slice(&addr.to_be_bytes());
        buf.extend_from_slice(&val.to_be_bytes());
    }

    write_cmd(port, &buf).ok_or("SET_FLASH_CMD send failed")?;
    if fw_ver >= 12 {
        let ack = read_byte(port).ok_or("SET_FLASH_CMD ACK timeout")?;
        if ack != 0x01 {
            return Err(format!("SET_FLASH_CMD ACK failed: 0x{ack:02x}"));
        }
    }
    Ok(())
}

/// Configure auto bank switching via DMG_SET_BANK_CHANGE_CMD (0xB8).
///
/// Sets up MBC5-style bank switching: write bank number to address 0x2100.
fn configure_bank_switching(port: &mut Box<dyn serialport::SerialPort>) -> Result<(), String> {
    let mut buf = [0u8; 7];
    buf[0] = DMG_SET_BANK_CHANGE_CMD;
    buf[1] = 1; // 1 command
    buf[2..6].copy_from_slice(&0x2100u32.to_be_bytes()); // address
    buf[6] = 0; // type = address mode (bank number goes in value)

    write_cmd(port, &buf).ok_or("DMG_SET_BANK_CHANGE_CMD send failed")?;
    let ack = read_byte(port).ok_or("DMG_SET_BANK_CHANGE_CMD ACK timeout")?;
    if ack != 0x01 {
        return Err(format!("DMG_SET_BANK_CHANGE_CMD ACK failed: 0x{ack:02x}"));
    }
    Ok(())
}

/// Erase the entire flash chip using the AMD chip erase command sequence.
///
/// Sends the 6-byte erase sequence, then polls DQ7 until the chip reports
/// erase complete (address 0 reads 0xFF).
fn chip_erase_amd(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Result<(), String> {
    // AMD chip erase: 6-command sequence
    cart_write_flash(
        port,
        &[
            (0x0AAA, 0x00AA), // unlock 1
            (0x0555, 0x0055), // unlock 2
            (0x0AAA, 0x0080), // erase setup
            (0x0AAA, 0x00AA), // unlock 1
            (0x0555, 0x0055), // unlock 2
            (0x0AAA, 0x0010), // chip erase
        ],
    )
    .ok_or("Chip erase command failed")?;

    // Poll for erase completion: read address 0, wait for 0xFF
    // Chip erase can take up to 60 seconds
    let timeout = Duration::from_secs(60);
    let start = std::time::Instant::now();

    loop {
        std::thread::sleep(Duration::from_millis(500));

        if let Some(data) = read_rom_bytes(port, fw_ver, 0, 2) {
            if data[0] == 0xFF {
                return Ok(());
            }
        }

        if start.elapsed() > timeout {
            return Err("Chip erase timed out after 60 seconds".to_string());
        }
    }
}

/// Flash chip parameters, queried via CFI (Common Flash Interface).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlashInfo {
    /// Raw flash ID bytes (manufacturer at [0], device at [2]).
    pub chip_id: Vec<u8>,
    /// Total flash size in bytes.
    pub size: u32,
    /// Write buffer size in bytes (0 = unbuffered writes only).
    pub buffer_size: u32,
    /// Whether chip erase is supported.
    pub chip_erase: bool,
    /// Whether sector erase is supported.
    pub sector_erase: bool,
    /// Erase sector regions: (sector_size, sector_count) pairs.
    pub sectors: Vec<(u32, u32)>,
}

#[allow(dead_code)]
impl FlashInfo {
    pub fn size_display(&self) -> String {
        format_size(self.size)
    }

    pub fn write_method(&self) -> u8 {
        if self.buffer_size > 0 { 0x02 } else { 0x01 } // buffered vs unbuffered
    }
}
