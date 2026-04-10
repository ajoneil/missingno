use std::io::{Read, Write};
use std::time::Duration;

use serialport::SerialPortType;

const GBXCART_VID: u16 = 0x1A86;
const GBXCART_PID: u16 = 0x7523;
const DEFAULT_BAUD: u32 = 1_000_000;
const QUERY_TIMEOUT: Duration = Duration::from_millis(100);

// Original firmware commands
const OFW_PCB_VER: u8 = 0x68;
const OFW_FW_VER: u8 = 0x56;

// Custom firmware commands
const QUERY_FW_INFO: u8 = 0xA1;

#[derive(Debug, Clone)]
pub struct DetectedDevice {
    pub port_name: String,
    pub device_name: String,
    pub pcb_version: u8,
    pub firmware_version: u16,
}

impl DetectedDevice {
    pub fn display_name(&self) -> String {
        if self.device_name.is_empty() {
            format!("GBxCart RW (PCB v{}, FW v{})", self.pcb_version, self.firmware_version)
        } else {
            self.device_name.clone()
        }
    }
}

/// Scan serial ports for GBxCart RW devices and query their firmware for identification.
///
/// This opens each matching port briefly to read the device name from the firmware.
/// Designed to be called from a background thread via `smol::unblock`.
pub fn detect_devices() -> Vec<DetectedDevice> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };

    ports
        .into_iter()
        .filter_map(|port| {
            if let SerialPortType::UsbPort(usb) = &port.port_type {
                if usb.vid == GBXCART_VID && usb.pid == GBXCART_PID {
                    return query_device(&port.port_name);
                }
            }
            None
        })
        .collect()
}

/// Open a serial port and query the GBxCart firmware for device info.
fn query_device(port_name: &str) -> Option<DetectedDevice> {
    let mut port = serialport::new(port_name, DEFAULT_BAUD)
        .timeout(QUERY_TIMEOUT)
        .open()
        .ok()?;

    port.clear(serialport::ClearBuffer::All).ok();

    // Query PCB version
    port.write_all(&[OFW_PCB_VER]).ok()?;
    port.flush().ok()?;
    let pcb_version = read_byte(&mut port)?;

    // Query original firmware version
    port.write_all(&[OFW_FW_VER]).ok()?;
    port.flush().ok()?;
    let ofw_ver = read_byte(&mut port)?;

    // Not a GBxCart RW if PCB >= 5 and OFW version is 0
    if pcb_version >= 5 && ofw_ver == 0 {
        return None;
    }

    // Query custom firmware info (only available on CFW devices)
    let (firmware_version, device_name) = query_firmware_info(&mut port).unwrap_or((0, String::new()));

    // If no CFW and no OFW, this isn't a device we recognise
    if firmware_version == 0 && ofw_ver == 0 {
        return None;
    }

    Some(DetectedDevice {
        port_name: port_name.to_string(),
        device_name,
        pcb_version,
        firmware_version,
    })
}

/// Query the custom firmware info struct (QUERY_FW_INFO, 0xA1).
///
/// Response format:
///   1 byte:  size (must be 8)
///   1 byte:  cfw_id (ASCII char, e.g. 'L')
///   2 bytes: fw_ver (u16, big-endian)
///   1 byte:  pcb_ver (u8)
///   4 bytes: fw_timestamp (u32, big-endian)
///
/// For fw_ver >= 12, additional data follows:
///   1 byte:  name_size
///   N bytes: device name (UTF-8)
fn query_firmware_info(port: &mut Box<dyn serialport::SerialPort>) -> Option<(u16, String)> {
    port.write_all(&[QUERY_FW_INFO]).ok()?;
    port.flush().ok()?;

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
                    // Strip null terminator if present
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

fn read_byte(port: &mut Box<dyn serialport::SerialPort>) -> Option<u8> {
    let mut buf = [0u8; 1];
    port.read_exact(&mut buf).ok()?;
    Some(buf[0])
}
