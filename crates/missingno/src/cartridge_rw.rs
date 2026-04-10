use std::io::{Read, Write};
use std::time::Duration;

use serialport::SerialPortType;

const GBXCART_VID: u16 = 0x1A86;
const GBXCART_PID: u16 = 0x7523;
const DEFAULT_BAUD: u32 = 1_000_000;
const QUERY_TIMEOUT: Duration = Duration::from_millis(100);

// Original firmware commands (used during initial handshake)
const OFW_PCB_VER: u8 = 0x68;

// Custom firmware commands
const QUERY_FW_INFO: u8 = 0xA1;
const SET_MODE_DMG: u8 = 0xA3;
const SET_VOLTAGE_5V: u8 = 0xA5;
const SET_VARIABLE: u8 = 0xA6;
const DISABLE_PULLUPS: u8 = 0xAC;

// DMG commands
const DMG_CART_READ: u8 = 0xB1;
const DMG_CART_WRITE: u8 = 0xB2;
const DMG_MBC_RESET: u8 = 0xB4;

// Flash commands
const CART_WRITE_FLASH_CMD: u8 = 0xD4;

// Power commands
const CART_PWR_ON: u8 = 0xF2;
const CART_PWR_OFF: u8 = 0xF3;
const QUERY_CART_PWR: u8 = 0xF4;

// Cleanup commands
const SET_ADDR_AS_INPUTS: u8 = 0xA8;
const OFW_DONE_LED_ON: u8 = 0x3D;

// Header reading constants
const HEADER_SIZE: usize = 0x180;
const CHUNK_SIZE: u16 = 64;

/// ROM size table: index (byte 0x148) → size in bytes.
const ROM_SIZES: &[(u8, u32)] = &[
    (0x00, 32 * 1024),
    (0x01, 64 * 1024),
    (0x02, 128 * 1024),
    (0x03, 256 * 1024),
    (0x04, 512 * 1024),
    (0x05, 1024 * 1024),
    (0x06, 2 * 1024 * 1024),
    (0x07, 4 * 1024 * 1024),
    (0x08, 8 * 1024 * 1024),
];

/// RAM size table: index (byte 0x149) → size in bytes.
const RAM_SIZES: &[(u8, u32)] = &[
    (0x00, 0),
    (0x01, 2 * 1024),
    (0x02, 8 * 1024),
    (0x03, 32 * 1024),
    (0x04, 128 * 1024),
    (0x05, 64 * 1024),
];

/// Known Nintendo logo bytes at 0x104-0x133.
const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00,
    0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD,
    0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB,
    0xB9, 0x33, 0x3E,
];

#[derive(Debug, Clone)]
pub struct DetectedDevice {
    pub port_name: String,
    pub device_name: String,
    pub pcb_version: u8,
    pub firmware_version: u16,
    pub cartridge: Option<CartridgeHeader>,
}

impl DetectedDevice {
    pub fn display_name(&self) -> String {
        if self.device_name.is_empty() {
            format!(
                "GBxCart RW (PCB v{}, FW v{})",
                self.pcb_version, self.firmware_version
            )
        } else {
            self.device_name.clone()
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CartridgeHeader {
    pub title: String,
    pub mapper_byte: u8,
    pub mapper_name: &'static str,
    pub rom_size: u32,
    pub ram_size: u32,
    pub has_battery: bool,
    pub sgb_flag: bool,
    pub header_checksum_valid: bool,
    /// Flash chip ID bytes, if a flash chip was detected.
    /// Manufacturer ID at index 0, device ID at index 2.
    pub flash_id: Option<Vec<u8>>,
}

/// Cheap check: list port names of connected GBxCart RW devices without opening them.
///
/// Returns a sorted list of port names (e.g. `["/dev/ttyUSB0"]`).
/// Used for polling to detect connect/disconnect — only triggers a full
/// `detect_devices()` query when this list changes.
pub fn list_ports() -> Vec<String> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };

    let mut names: Vec<String> = ports
        .into_iter()
        .filter_map(|port| {
            if let SerialPortType::UsbPort(usb) = &port.port_type {
                if usb.vid == GBXCART_VID && usb.pid == GBXCART_PID {
                    return Some(port.port_name);
                }
            }
            None
        })
        .collect();
    names.sort();
    names
}

/// Query specific ports for GBxCart RW devices.
///
/// Only queries the given port names, not all available ports.
/// Designed to be called from a background thread via `smol::unblock`.
pub fn detect_ports(port_names: &[String]) -> Vec<DetectedDevice> {
    port_names
        .iter()
        .filter_map(|name| query_device(name))
        .collect()
}

// ── Device query ─────────────────────────────────────────────────────

/// Open a serial port and query the GBxCart firmware for device info,
/// then attempt to read the cartridge header if one is inserted.
fn query_device(port_name: &str) -> Option<DetectedDevice> {
    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(QUERY_TIMEOUT)
        .open()
        .ok()?;

    port.clear(serialport::ClearBuffer::All).ok();

    // Query PCB version (legacy command, always available)
    write_cmd(&mut port, &[OFW_PCB_VER])?;
    let pcb_version = read_byte(&mut port)?;

    // Query firmware info — we require the Lesserkuma firmware (v12+)
    let (firmware_version, device_name) = query_firmware_info(&mut port)?;
    if firmware_version < 12 {
        return None;
    }

    // Try to read the cartridge header
    let cartridge = read_cartridge_header(&mut port, firmware_version);

    // Clean up: safe pin state, power off cart, light done LED
    cleanup(&mut port, firmware_version);

    Some(DetectedDevice {
        port_name: port_name.to_string(),
        device_name,
        pcb_version,
        firmware_version,
        cartridge,
    })
}

/// Put the device back in a clean state: safe pins, power off cartridge, done LED.
fn cleanup(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) {
    let _ = write_cmd_ack(port, &[SET_ADDR_AS_INPUTS], fw_ver);
    if fw_ver >= 12 {
        let _ = write_cmd_ack(port, &[CART_PWR_OFF], fw_ver);
    }
    let _ = write_cmd(port, &[OFW_DONE_LED_ON]);
    // Flush everything so the next connection starts clean
    let _ = port.flush();
    let _ = port.clear(serialport::ClearBuffer::All);
}

// ── Firmware info ────────────────────────────────────────────────────

/// Query the custom firmware info struct (QUERY_FW_INFO, 0xA1).
fn query_firmware_info(port: &mut Box<dyn serialport::SerialPort>) -> Option<(u16, String)> {
    write_cmd(port, &[QUERY_FW_INFO])?;

    let size = read_byte(port)?;
    if size != 8 {
        return None;
    }

    let mut info = [0u8; 8];
    port.read_exact(&mut info).ok()?;

    // Parse: >cHBI = (char, u16 BE, u8, u32 BE)
    let fw_ver = u16::from_be_bytes([info[1], info[2]]);

    let mut device_name = String::new();

    if fw_ver >= 12 {
        if let Some(name_size) = read_byte(port) {
            if name_size > 0 {
                let mut name_buf = vec![0u8; name_size as usize];
                if port.read_exact(&mut name_buf).is_ok() {
                    if let Some(null_pos) = name_buf.iter().position(|&b| b == 0) {
                        name_buf.truncate(null_pos);
                    }
                    device_name = String::from_utf8_lossy(&name_buf).into_owned();
                }
            }
        }
    }

    Some((fw_ver, device_name))
}

// ── Cartridge header reading ─────────────────────────────────────────

/// Set up the device for DMG cartridge access: enter mode, power on, disable pullups, reset MBC.
fn enter_dmg_mode(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Option<()> {
    port.clear(serialport::ClearBuffer::Input).ok();

    // SetMode("DMG")
    write_cmd_ack(port, &[SET_MODE_DMG], fw_ver)?;
    write_cmd_ack(port, &[SET_VOLTAGE_5V], fw_ver)?;
    set_variable(port, fw_ver, 1, 0x0B, 1)?; // DMG_READ_METHOD = A15
    set_variable(port, fw_ver, 1, 0x00, 1)?; // CART_MODE = DMG
    set_variable(port, fw_ver, 4, 0x00, 0)?; // ADDRESS = 0

    // Power on cartridge
    cart_power_on(port, fw_ver)?;

    // ReadInfo setup
    if fw_ver >= 8 {
        write_cmd_ack(port, &[DISABLE_PULLUPS], fw_ver)?;
    }
    write_cmd_ack(port, &[SET_VOLTAGE_5V], fw_ver)?;
    write_cmd_ack(port, &[DMG_MBC_RESET], fw_ver)?;

    // Clear CS pulse flags
    set_variable(port, fw_ver, 1, 0x08, 0)?; // DMG_READ_CS_PULSE = 0
    set_variable(port, fw_ver, 1, 0x09, 0)?; // DMG_WRITE_CS_PULSE = 0

    Some(())
}

/// Enter DMG mode and read the first 0x180 bytes from the cartridge.
fn read_cartridge_header(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
) -> Option<CartridgeHeader> {
    eprintln!("[cartridge_rw] reading header (fw_ver={fw_ver})");

    port.set_timeout(Duration::from_millis(500)).ok()?;

    if enter_dmg_mode(port, fw_ver).is_none() {
        eprintln!("[cartridge_rw] DMG mode setup failed");
        return None;
    }
    eprintln!("[cartridge_rw] DMG mode ready");

    // Configure for header read
    set_variable(port, fw_ver, 2, 0x00, CHUNK_SIZE as u32)?; // TRANSFER_SIZE
    set_variable(port, fw_ver, 4, 0x00, 0)?; // ADDRESS = 0
    set_variable(port, fw_ver, 1, 0x01, 1)?; // DMG_ACCESS_MODE = ROM_READ

    // 7. Read 0x180 bytes in chunks
    let mut header = vec![0u8; HEADER_SIZE];
    let chunks = HEADER_SIZE / CHUNK_SIZE as usize;
    for i in 0..chunks {
        if write_cmd(port, &[DMG_CART_READ]).is_none() {
            eprintln!("[cartridge_rw] DMG_CART_READ send failed at chunk {i}");
            return None;
        }
        let offset = i * CHUNK_SIZE as usize;
        if port
            .read_exact(&mut header[offset..offset + CHUNK_SIZE as usize])
            .is_err()
        {
            eprintln!("[cartridge_rw] read failed at chunk {i}");
            return None;
        }
    }

    // Log the first bytes for debugging
    eprintln!(
        "[cartridge_rw] header read complete, first 16 bytes: {:02x?}",
        &header[..16]
    );
    eprintln!(
        "[cartridge_rw] logo bytes (0x104-0x134): {:02x?}",
        &header[0x104..0x134]
    );
    eprintln!(
        "[cartridge_rw] title bytes (0x134-0x144): {:02x?}",
        &header[0x134..0x144]
    );
    eprintln!(
        "[cartridge_rw] mapper=0x{:02x} rom_size=0x{:02x} ram_size=0x{:02x}",
        header[0x147], header[0x148], header[0x149]
    );

    let mut result = parse_cartridge_header(&header);

    // Probe for flash chip while the cart is still powered and in DMG mode
    if let Some(cart) = &mut result {
        cart.flash_id = detect_flash(port, fw_ver);
        eprintln!(
            "[cartridge_rw] parsed: \"{}\" ({}) flashable={}",
            cart.title, cart.mapper_name, cart.flashable()
        );
    } else {
        eprintln!("[cartridge_rw] header parse failed (logo mismatch or invalid)");
    }
    result
}

/// Power on the cartridge slot with the proper handshake.
///
/// Sends CART_PWR_ON, waits for ACK with polling, then verifies power state.
fn cart_power_on(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Option<()> {
    // Check if already powered
    write_cmd(port, &[QUERY_CART_PWR])?;
    let pwr = read_byte(port)?;
    eprintln!("[cartridge_rw] cart power state: {pwr}");
    if pwr == 1 {
        return Some(());
    }

    // Send mode again before power-on (as per FlashGBX sequence)
    write_cmd_ack(port, &[SET_MODE_DMG], fw_ver)?;

    // Send CART_PWR_ON and wait for ACK
    write_cmd(port, &[CART_PWR_ON])?;
    std::thread::sleep(Duration::from_millis(200));

    // Poll for ACK: wait up to 1000ms
    let mut got_ack = false;
    for attempt in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        let mut buf = [0u8; 64];
        match port.read(&mut buf) {
            Ok(n) => {
                eprintln!("[cartridge_rw] power ACK poll {attempt}: got {n} bytes: {:02x?}", &buf[..n]);
                if n > 0 && buf[n - 1] == 0x01 {
                    got_ack = true;
                    break;
                }
            }
            Err(e) => {
                eprintln!("[cartridge_rw] power ACK poll {attempt}: {e}");
            }
        }
    }

    if !got_ack {
        eprintln!("[cartridge_rw] cart power on: no ACK received");
        return None;
    }

    // Verify power is on
    write_cmd(port, &[QUERY_CART_PWR])?;
    let pwr = read_byte(port)?;
    eprintln!("[cartridge_rw] cart power verify: {pwr}");
    if pwr != 1 {
        return None;
    }

    Some(())
}

/// Parse a raw 0x180-byte header into a CartridgeHeader.
fn parse_cartridge_header(header: &[u8]) -> Option<CartridgeHeader> {
    if header.len() < HEADER_SIZE {
        return None;
    }

    // Validate Nintendo logo
    if header[0x104..0x134] != NINTENDO_LOGO {
        return None;
    }

    let (title, sgb_flag, has_battery) = missingno_gb::cartridge::parse_header(header);
    let mapper_byte = header[0x147];
    let rom_size_index = header[0x148];
    let ram_size_index = header[0x149];

    // Validate header checksum
    let mut checksum: u8 = 0;
    for &byte in &header[0x134..0x14D] {
        checksum = checksum.wrapping_sub(byte).wrapping_sub(1);
    }
    let header_checksum_valid = checksum == header[0x14D];

    let rom_size = ROM_SIZES
        .iter()
        .find(|(i, _)| *i == rom_size_index)
        .map(|(_, s)| *s)
        .unwrap_or(0);

    let ram_size = if mapper_byte == 0x05 || mapper_byte == 0x06 {
        // MBC2: fixed 512 bytes
        512
    } else {
        RAM_SIZES
            .iter()
            .find(|(i, _)| *i == ram_size_index)
            .map(|(_, s)| *s)
            .unwrap_or(0)
    };

    let mapper_name = mapper_name(mapper_byte);

    Some(CartridgeHeader {
        title,
        mapper_byte,
        mapper_name,
        rom_size,
        ram_size,
        has_battery,
        sgb_flag,
        header_checksum_valid,
        flash_id: None, // Set by detect_flash() after header read
    })
}

fn mapper_name(byte: u8) -> &'static str {
    match byte {
        0x00 | 0x08 | 0x09 => "No MBC",
        0x01..=0x03 => "MBC1",
        0x05 | 0x06 => "MBC2",
        0x0f..=0x13 => "MBC3",
        0x19..=0x1e => "MBC5",
        0x20 => "MBC6",
        0x22 => "MBC7",
        0xfe => "HuC-3",
        0xff => "HuC-1",
        _ => "Unknown",
    }
}

// ── Flash detection ──────────────────────────────────────────────────

/// Probe for a flash chip using the standard AMD/JEDEC ID command sequence.
///
/// Reads 8 bytes from address 0, sends the flash ID command via the flash write
/// pin (WR), reads again. If the data changed, a flash chip responded — the
/// cartridge is flashable. Resets the flash chip back to read mode afterward.
///
/// This is safe on commercial cartridges: the writes target addresses (0x0AAA,
/// 0x0555) that MBC chips don't decode, and use the flash write pin which has
/// no effect on standard ROM chips.
/// Returns the flash ID bytes if a flash chip is detected, or None for a standard ROM cart.
fn detect_flash(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) -> Option<Vec<u8>> {
    // Set the flash write-enable pin to WR (pin mode 1)
    set_variable(port, fw_ver, 1, 0x04, 1)?;

    // Read original ROM data at address 0
    let original = read_rom_bytes(port, fw_ver, 0, 8)?;

    // Send AMD flash ID command sequence via CART_WRITE_FLASH_CMD
    let id_cmd: &[(u32, u16)] = &[
        (0x0AAA, 0x00AA),
        (0x0555, 0x0055),
        (0x0AAA, 0x0090),
    ];
    cart_write_flash(port, id_cmd)?;

    // Read back — if flash chip present, this returns the chip ID instead of ROM data
    let probe = read_rom_bytes(port, fw_ver, 0, 8)?;

    // Reset flash back to read mode
    let reset_cmd: &[(u32, u16)] = &[(0x0000, 0x00F0)];
    let _ = cart_write_flash(port, reset_cmd);

    if original != probe {
        eprintln!(
            "[cartridge_rw] flash detected: ROM={:02x?} ID={:02x?}",
            &original[..4],
            &probe[..4]
        );
        Some(probe)
    } else {
        None
    }
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

/// Send flash command sequence via CART_WRITE_FLASH_CMD (0xD4).
///
/// Each command is (address, value) where value is u16 (big-endian).
/// This uses the flash write pin rather than the normal cart bus.
fn cart_write_flash(
    port: &mut Box<dyn serialport::SerialPort>,
    commands: &[(u32, u16)],
) -> Option<()> {
    let num = commands.len() as u8;
    let mut buf = Vec::with_capacity(3 + num as usize * 6);
    buf.push(CART_WRITE_FLASH_CMD);
    buf.push(0x00); // not a flashcart write (just probing)
    buf.push(num);
    for &(addr, val) in commands {
        buf.extend_from_slice(&addr.to_be_bytes());
        buf.extend_from_slice(&val.to_be_bytes());
    }
    write_cmd(port, &buf)?;
    // Read ACK
    let ack = read_byte(port)?;
    if ack != 0x01 {
        return None;
    }
    Some(())
}

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
                (0x6000, 0x01u8),                       // Mode 1 (advanced banking)
                (0x2000, (bank & 0x1F) as u8),           // Lower 5 bits
                (0x4000, ((bank >> 5) & 0x03) as u8),    // Upper 2 bits
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
                (0x3000, ((bank >> 8) & 0x01) as u8),    // High bit first
                (0x2100, (bank & 0xFF) as u8),            // Low 8 bits
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
    write_cmd(&mut port, &[OFW_PCB_VER])
        .ok_or("Failed to query PCB version")?;
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
    let num_banks = if no_mbc { 1 } else { rom_size / ROM_BANK_SIZE as usize };
    let mut rom = Vec::with_capacity(rom_size);

    eprintln!(
        "[cartridge_rw] dumping ROM: {} bytes, {} banks, mapper 0x{:02x}",
        rom_size, num_banks, header.mapper_byte
    );

    for bank in 0..num_banks as u32 {
        // Bank switch
        let (writes, start_addr) = select_rom_bank(header.mapper_byte, bank);
        for (addr, val) in writes {
            cart_write(&mut port, fw_ver, addr, val)
                .ok_or_else(|| format!("Bank switch failed at bank {bank}"))?;
        }

        // Set up read for this bank
        set_variable(&mut port, fw_ver, 2, 0x00, bulk_chunk)
            .ok_or("Set TRANSFER_SIZE failed")?;
        set_variable(&mut port, fw_ver, 4, 0x00, start_addr)
            .ok_or("Set ADDRESS failed")?;
        set_variable(&mut port, fw_ver, 1, 0x01, 1)
            .ok_or("Set DMG_ACCESS_MODE failed")?;

        let mut remaining = if no_mbc { rom_size } else { ROM_BANK_SIZE as usize };
        while remaining > 0 {
            let chunk = remaining.min(bulk_chunk as usize);
            // Update transfer size if last chunk is smaller
            if chunk != bulk_chunk as usize {
                set_variable(&mut port, fw_ver, 2, 0x00, chunk as u32)
                    .ok_or("Set TRANSFER_SIZE for final chunk failed")?;
            }

            write_cmd(&mut port, &[DMG_CART_READ])
                .ok_or("DMG_CART_READ send failed")?;

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

    eprintln!("[cartridge_rw] dump complete: {} bytes", rom.len());
    Ok(rom)
}

// ── Protocol helpers ─────────────────────────────────────────────────

fn write_cmd(port: &mut Box<dyn serialport::SerialPort>, data: &[u8]) -> Option<()> {
    port.write_all(data).ok()?;
    port.flush().ok()
}

fn write_cmd_ack(
    port: &mut Box<dyn serialport::SerialPort>,
    data: &[u8],
    fw_ver: u16,
) -> Option<()> {
    write_cmd(port, data)?;
    if fw_ver >= 12 {
        match read_byte(port) {
            Some(0x01) | Some(0x03) => {}
            Some(other) => {
                eprintln!(
                    "[cartridge_rw] ACK failed for cmd 0x{:02x}: got 0x{:02x}",
                    data[0], other
                );
                return None;
            }
            None => {
                eprintln!(
                    "[cartridge_rw] ACK failed for cmd 0x{:02x}: read timeout",
                    data[0]
                );
                return None;
            }
        }
    }
    Some(())
}

/// DMG_CART_WRITE: [0xB2, addr(4B BE), value(1B)]
fn cart_write(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
    address: u32,
    value: u8,
) -> Option<()> {
    let mut buf = [0u8; 6];
    buf[0] = DMG_CART_WRITE;
    buf[1..5].copy_from_slice(&address.to_be_bytes());
    buf[5] = value;
    write_cmd_ack(port, &buf, fw_ver)
}

/// SET_VARIABLE: [0xA6, size, key(4B BE), value(4B BE)]
fn set_variable(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
    size: u8,
    key: u32,
    value: u32,
) -> Option<()> {
    let mut buf = [0u8; 10];
    buf[0] = SET_VARIABLE;
    buf[1] = size;
    buf[2..6].copy_from_slice(&key.to_be_bytes());
    buf[6..10].copy_from_slice(&value.to_be_bytes());
    write_cmd_ack(port, &buf, fw_ver)
}

fn read_byte(port: &mut Box<dyn serialport::SerialPort>) -> Option<u8> {
    let mut buf = [0u8; 1];
    port.read_exact(&mut buf).ok()?;
    Some(buf[0])
}

pub fn format_size(bytes: u32) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else if bytes > 0 {
        format!("{} bytes", bytes)
    } else {
        "None".to_string()
    }
}

impl CartridgeHeader {
    pub fn flashable(&self) -> bool {
        self.flash_id.is_some()
    }

    pub fn rom_size_display(&self) -> String {
        format_size(self.rom_size)
    }

    pub fn ram_size_display(&self) -> String {
        format_size(self.ram_size)
    }
}
