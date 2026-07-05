use std::io::{Read, Write};
use std::time::Duration;

use super::detect::{DEFAULT_BAUD, cleanup, enter_dmg_mode, query_firmware_info};
use super::detect::CartridgeHeader;
use super::protocol::{
    DMG_CART_READ, DMG_CART_WRITE_SRAM, OFW_PCB_VER, cart_write, read_byte, set_variable, write_cmd,
};

// ── Bank switching ───────────────────────────────────────────────────

const ROM_BANK_SIZE: u32 = 0x4000;

/// Return the register writes needed to select a ROM bank, and the read start address.
fn select_rom_bank(mapper_byte: u8, bank: u32) -> (Vec<(u32, u8)>, u32) {
    match mapper_byte {
        // No MBC — single 32K, no bank switching
        0x00 | 0x08 | 0x09 => (vec![], 0x0000),

        // MBC1
        0x01..=0x03 => {
            let writes = vec![
                (0x6000, 0x01u8),                     // Mode 1 (advanced banking)
                (0x2000, (bank & 0x1F) as u8),        // Lower 5 bits
                (0x4000, ((bank >> 5) & 0x03) as u8), // Upper 2 bits
            ];
            let addr = if bank & 0x1F == 0 { 0x0000 } else { 0x4000 };
            (writes, addr)
        }

        // MBC2
        0x05 | 0x06 => {
            let writes = vec![(0x2100, (bank & 0xFF) as u8)];
            let addr = if bank == 0 { 0x0000 } else { 0x4000 };
            (writes, addr)
        }

        // MBC3
        0x0f..=0x13 => {
            let writes = vec![(0x2100, (bank & 0xFF) as u8)];
            let addr = if bank == 0 { 0x0000 } else { 0x4000 };
            (writes, addr)
        }

        // MBC5 — 9-bit bank number, high byte first
        0x19..=0x1e => {
            let writes = vec![
                (0x3000, ((bank >> 8) & 0x01) as u8), // High bit first
                (0x2100, (bank & 0xFF) as u8),        // Low 8 bits
            ];
            let addr = if bank == 0 { 0x0000 } else { 0x4000 };
            (writes, addr)
        }

        // Unsupported MBC — try basic bank switching
        _ => {
            let writes = vec![(0x2100, (bank & 0xFF) as u8)];
            let addr = if bank == 0 { 0x0000 } else { 0x4000 };
            (writes, addr)
        }
    }
}

// ── ROM dumping ──────────────────────────────────────────────────────

/// Progress update during a ROM dump.
#[derive(Debug, Clone)]
pub struct DumpProgress {
    pub bytes_done: usize,
    pub bytes_total: usize,
}

/// Dump the full ROM from a cartridge. Opens the serial port, reads all banks,
/// and returns the complete ROM data.
///
/// Designed to be called from a background thread via `smol::unblock`.
pub fn dump_rom(
    port_name: &str,
    header: &CartridgeHeader,
    progress: &mut dyn FnMut(DumpProgress),
) -> Result<Vec<u8>, String> {
    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(Duration::from_millis(2000))
        .open()
        .map_err(|e| format!("Failed to open port: {e}"))?;

    port.clear(serialport::ClearBuffer::All).ok();

    // We require Lesserkuma firmware v12+
    write_cmd(&mut port, &[OFW_PCB_VER]).ok_or("Failed to query PCB version")?;
    let _pcb = read_byte(&mut port).ok_or("No PCB version response")?;
    let (fw_ver, _) = query_firmware_info(&mut port)
        .ok_or("Failed to query firmware — is this a GBxCart RW with Lesserkuma firmware?")?;
    if fw_ver < 12 {
        return Err(format!("Firmware v{fw_ver} too old, need v12+"));
    }

    enter_dmg_mode(&mut port, fw_ver).ok_or("DMG mode setup failed")?;

    let bulk_chunk: u32 = 0x1000; // 4096 bytes per transfer

    let rom_size = header.rom_size as usize;
    let no_mbc = matches!(header.mapper_byte, 0x00 | 0x08 | 0x09);
    // No MBC: one flat 32K read. MBC: 16K banks.
    let num_banks = if no_mbc {
        1
    } else {
        rom_size / ROM_BANK_SIZE as usize
    };
    let mut rom = Vec::with_capacity(rom_size);

    for bank in 0..num_banks as u32 {
        // Bank switch
        let (writes, start_addr) = select_rom_bank(header.mapper_byte, bank);
        for (addr, val) in writes {
            cart_write(&mut port, fw_ver, addr, val)
                .ok_or_else(|| format!("Bank switch failed at bank {bank}"))?;
        }

        // Set up read for this bank
        set_variable(&mut port, fw_ver, 2, 0x00, bulk_chunk).ok_or("Set TRANSFER_SIZE failed")?;
        set_variable(&mut port, fw_ver, 4, 0x00, start_addr).ok_or("Set ADDRESS failed")?;
        set_variable(&mut port, fw_ver, 1, 0x01, 1).ok_or("Set DMG_ACCESS_MODE failed")?;

        let mut remaining = if no_mbc {
            rom_size
        } else {
            ROM_BANK_SIZE as usize
        };
        while remaining > 0 {
            let chunk = remaining.min(bulk_chunk as usize);
            // Update transfer size if last chunk is smaller
            if chunk != bulk_chunk as usize {
                set_variable(&mut port, fw_ver, 2, 0x00, chunk as u32)
                    .ok_or("Set TRANSFER_SIZE for final chunk failed")?;
            }

            write_cmd(&mut port, &[DMG_CART_READ]).ok_or("DMG_CART_READ send failed")?;

            let mut buf = vec![0u8; chunk];
            port.read_exact(&mut buf)
                .map_err(|e| format!("Read failed at bank {bank}: {e}"))?;
            rom.extend_from_slice(&buf);
            remaining -= chunk;

            progress(DumpProgress {
                bytes_done: rom.len(),
                bytes_total: rom_size,
            });
        }
    }

    // Cleanup
    cleanup(&mut port, fw_ver);

    Ok(rom)
}

// ── SRAM read/write ──────────────────────────────────────────────────

const RAM_BANK_SIZE: usize = 0x2000; // 8 KB per bank

/// Read SRAM from the cartridge.
///
/// Returns the save data bytes. For MBC2, each byte is masked to 4 bits.
/// Designed to be called from a background thread via `smol::unblock`.
pub fn read_sram(port_name: &str, header: &CartridgeHeader) -> Result<Vec<u8>, String> {
    if header.ram_size == 0 {
        return Err("Cartridge has no SRAM".to_string());
    }

    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(Duration::from_millis(2000))
        .open()
        .map_err(|e| format!("Failed to open port: {e}"))?;

    port.clear(serialport::ClearBuffer::All).ok();

    write_cmd(&mut port, &[OFW_PCB_VER]).ok_or("Failed to query PCB version")?;
    let _pcb = read_byte(&mut port).ok_or("No PCB version response")?;
    let (fw_ver, _) = query_firmware_info(&mut port).ok_or("Failed to query firmware")?;
    if fw_ver < 12 {
        return Err(format!("Firmware v{fw_ver} too old, need v12+"));
    }

    enter_dmg_mode(&mut port, fw_ver).ok_or("DMG mode setup failed")?;

    // Enable RAM: write 0x0A to 0x0000
    cart_write(&mut port, fw_ver, 0x0000, 0x0A).ok_or("Enable RAM failed")?;

    // MBC1: set mode 1 for multi-bank RAM access
    if matches!(header.mapper_byte, 0x01..=0x03) {
        cart_write(&mut port, fw_ver, 0x6000, 0x01).ok_or("MBC1 mode 1 failed")?;
    }

    let ram_size = if matches!(header.mapper_byte, 0x05 | 0x06) {
        512 // MBC2: 512 x 4-bit
    } else {
        header.ram_size as usize
    };
    let num_banks = (ram_size / RAM_BANK_SIZE).max(1);
    let bank_size = if ram_size < RAM_BANK_SIZE {
        ram_size
    } else {
        RAM_BANK_SIZE
    };
    let chunk_size: u16 = 512;

    let mut sram = Vec::with_capacity(ram_size);

    for bank in 0..num_banks {
        // Select RAM bank
        cart_write(&mut port, fw_ver, 0x4000, bank as u8)
            .ok_or_else(|| format!("RAM bank select failed for bank {bank}"))?;

        set_variable(&mut port, fw_ver, 2, 0x00, chunk_size as u32)
            .ok_or("Set TRANSFER_SIZE failed")?;
        set_variable(&mut port, fw_ver, 4, 0x00, 0xA000).ok_or("Set ADDRESS failed")?;
        set_variable(&mut port, fw_ver, 1, 0x01, 3).ok_or("Set DMG_ACCESS_MODE failed")?;
        set_variable(&mut port, fw_ver, 1, 0x08, 1).ok_or("Set DMG_READ_CS_PULSE failed")?;

        let mut remaining = bank_size;
        while remaining > 0 {
            let n = remaining.min(chunk_size as usize);
            if n != chunk_size as usize {
                set_variable(&mut port, fw_ver, 2, 0x00, n as u32)
                    .ok_or("Set TRANSFER_SIZE failed")?;
            }
            write_cmd(&mut port, &[DMG_CART_READ]).ok_or("DMG_CART_READ failed")?;
            let mut buf = vec![0u8; n];
            port.read_exact(&mut buf)
                .map_err(|e| format!("SRAM read failed: {e}"))?;
            sram.extend_from_slice(&buf);
            remaining -= n;
        }

        set_variable(&mut port, fw_ver, 1, 0x08, 0).ok_or("Clear DMG_READ_CS_PULSE failed")?;
    }

    // MBC2: mask to 4-bit
    if matches!(header.mapper_byte, 0x05 | 0x06) {
        for byte in &mut sram {
            *byte &= 0x0F;
        }
    }

    // Disable RAM
    cart_write(&mut port, fw_ver, 0x0000, 0x00).ok_or("Disable RAM failed")?;
    if matches!(header.mapper_byte, 0x01..=0x03) {
        let _ = cart_write(&mut port, fw_ver, 0x6000, 0x00);
    }

    cleanup(&mut port, fw_ver);

    Ok(sram)
}

/// Write SRAM to the cartridge.
///
/// Designed to be called from a background thread via `smol::unblock`.
pub fn write_sram(port_name: &str, header: &CartridgeHeader, data: &[u8]) -> Result<(), String> {
    if header.ram_size == 0 {
        return Err("Cartridge has no SRAM".to_string());
    }

    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(Duration::from_millis(2000))
        .open()
        .map_err(|e| format!("Failed to open port: {e}"))?;

    port.clear(serialport::ClearBuffer::All).ok();

    write_cmd(&mut port, &[OFW_PCB_VER]).ok_or("Failed to query PCB version")?;
    let _pcb = read_byte(&mut port).ok_or("No PCB version response")?;
    let (fw_ver, _) = query_firmware_info(&mut port).ok_or("Failed to query firmware")?;
    if fw_ver < 12 {
        return Err(format!("Firmware v{fw_ver} too old, need v12+"));
    }

    enter_dmg_mode(&mut port, fw_ver).ok_or("DMG mode setup failed")?;

    // Enable RAM
    cart_write(&mut port, fw_ver, 0x0000, 0x0A).ok_or("Enable RAM failed")?;

    if matches!(header.mapper_byte, 0x01..=0x03) {
        cart_write(&mut port, fw_ver, 0x6000, 0x01).ok_or("MBC1 mode 1 failed")?;
    }

    let ram_size = if matches!(header.mapper_byte, 0x05 | 0x06) {
        512
    } else {
        header.ram_size as usize
    };
    let num_banks = (ram_size / RAM_BANK_SIZE).max(1);
    let bank_size = if ram_size < RAM_BANK_SIZE {
        ram_size
    } else {
        RAM_BANK_SIZE
    };
    let chunk_size: u16 = 256; // MAX_BUFFER_WRITE

    // Pad or truncate data to match RAM size
    let mut padded = data.to_vec();
    padded.resize(ram_size, 0xFF);

    for bank in 0..num_banks {
        let bank_offset = bank * bank_size;
        let bank_data = &padded[bank_offset..bank_offset + bank_size];

        // Select RAM bank
        cart_write(&mut port, fw_ver, 0x4000, bank as u8)
            .ok_or_else(|| format!("RAM bank select failed for bank {bank}"))?;

        set_variable(&mut port, fw_ver, 2, 0x00, chunk_size as u32)
            .ok_or("Set TRANSFER_SIZE failed")?;
        set_variable(&mut port, fw_ver, 4, 0x00, 0xA000).ok_or("Set ADDRESS failed")?;
        set_variable(&mut port, fw_ver, 1, 0x01, 4).ok_or("Set DMG_ACCESS_MODE failed")?;
        set_variable(&mut port, fw_ver, 1, 0x09, 1).ok_or("Set DMG_WRITE_CS_PULSE failed")?;

        for chunk in bank_data.chunks(chunk_size as usize) {
            write_cmd(&mut port, &[DMG_CART_WRITE_SRAM]).ok_or("DMG_CART_WRITE_SRAM failed")?;
            port.write_all(chunk)
                .map_err(|e| format!("SRAM write failed: {e}"))?;
            port.flush().map_err(|e| format!("Flush failed: {e}"))?;

            let ack = read_byte(&mut port).ok_or("No ACK after SRAM write")?;
            if ack != 0x01 && ack != 0x03 {
                return Err(format!("SRAM write failed at bank {bank}: ACK=0x{ack:02x}"));
            }
        }

        set_variable(&mut port, fw_ver, 4, 0x00, 0).ok_or("Clear ADDRESS failed")?;
        set_variable(&mut port, fw_ver, 1, 0x09, 0).ok_or("Clear DMG_WRITE_CS_PULSE failed")?;
    }

    // Disable RAM
    cart_write(&mut port, fw_ver, 0x0000, 0x00).ok_or("Disable RAM failed")?;
    if matches!(header.mapper_byte, 0x01..=0x03) {
        let _ = cart_write(&mut port, fw_ver, 0x6000, 0x00);
    }

    cleanup(&mut port, fw_ver);

    Ok(())
}
