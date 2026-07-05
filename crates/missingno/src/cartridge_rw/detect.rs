use std::io::{Read, Write};
use std::time::Duration;

use serialport::SerialPortType;

use super::flash::{FlashInfo, detect_flash};
use super::format_size;
use super::protocol::{
    CART_PWR_OFF, CART_PWR_ON, DISABLE_PULLUPS, DMG_CART_READ, DMG_MBC_RESET, OFW_DONE_LED_ON,
    OFW_PCB_VER, QUERY_CART_PWR, QUERY_FW_INFO, SET_ADDR_AS_INPUTS, SET_MODE_DMG, SET_VOLTAGE_5V,
    read_byte, set_variable, write_cmd, write_cmd_ack,
};

const GBXCART_VID: u16 = 0x1A86;
const GBXCART_PID: u16 = 0x7523;
pub(super) const DEFAULT_BAUD: u32 = 1_000_000;
const QUERY_TIMEOUT: Duration = Duration::from_millis(100);

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
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
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
    /// Flash chip info, if a flash chip was detected.
    pub flash: Option<FlashInfo>,
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
pub(super) fn cleanup(port: &mut Box<dyn serialport::SerialPort>, fw_ver: u16) {
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
pub(super) fn query_firmware_info(
    port: &mut Box<dyn serialport::SerialPort>,
) -> Option<(u16, String)> {
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
pub(super) fn enter_dmg_mode(
    port: &mut Box<dyn serialport::SerialPort>,
    fw_ver: u16,
) -> Option<()> {
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
    port.set_timeout(Duration::from_millis(500)).ok()?;

    if enter_dmg_mode(port, fw_ver).is_none() {
        return None;
    }

    // Configure for header read
    set_variable(port, fw_ver, 2, 0x00, CHUNK_SIZE as u32)?; // TRANSFER_SIZE
    set_variable(port, fw_ver, 4, 0x00, 0)?; // ADDRESS = 0
    set_variable(port, fw_ver, 1, 0x01, 1)?; // DMG_ACCESS_MODE = ROM_READ

    // 7. Read 0x180 bytes in chunks
    let mut header = vec![0u8; HEADER_SIZE];
    let chunks = HEADER_SIZE / CHUNK_SIZE as usize;
    for i in 0..chunks {
        if write_cmd(port, &[DMG_CART_READ]).is_none() {
            return None;
        }
        let offset = i * CHUNK_SIZE as usize;
        if port
            .read_exact(&mut header[offset..offset + CHUNK_SIZE as usize])
            .is_err()
        {
            return None;
        }
    }

    let mut result = parse_cartridge_header(&header);

    // Always probe for flash — an erased cart has no valid header but is still flashable
    let flash = detect_flash(port, fw_ver);

    if let Some(cart) = &mut result {
        cart.flash = flash;
    } else if let Some(flash) = flash {
        // No valid header but flash chip detected — erased or empty flash cart
        result = Some(CartridgeHeader {
            title: String::new(),
            mapper_byte: 0,
            mapper_name: "Unknown",
            rom_size: 0,
            ram_size: 0,
            has_battery: false,
            sgb_flag: false,
            header_checksum_valid: false,
            flash: Some(flash),
        });
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
    for _attempt in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        let mut buf = [0u8; 64];
        match port.read(&mut buf) {
            Ok(n) => {
                if n > 0 && buf[n - 1] == 0x01 {
                    got_ack = true;
                    break;
                }
            }
            Err(_) => {}
        }
    }

    if !got_ack {
        return None;
    }

    // Verify power is on
    write_cmd(port, &[QUERY_CART_PWR])?;
    let pwr = read_byte(port)?;
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
        flash: None, // Set by detect_flash() after header read
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

impl CartridgeHeader {
    #[allow(dead_code)]
    pub fn flashable(&self) -> bool {
        self.flash.is_some()
    }

    pub fn rom_size_display(&self) -> String {
        format_size(self.rom_size)
    }

    pub fn ram_size_display(&self) -> String {
        format_size(self.ram_size)
    }
}
