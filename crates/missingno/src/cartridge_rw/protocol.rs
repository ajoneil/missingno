use std::io::{Read, Write};

// Original firmware commands (used during initial handshake)
pub(super) const OFW_PCB_VER: u8 = 0x68;

// Custom firmware commands
pub(super) const QUERY_FW_INFO: u8 = 0xA1;
pub(super) const SET_MODE_DMG: u8 = 0xA3;
pub(super) const SET_VOLTAGE_5V: u8 = 0xA5;
const SET_VARIABLE: u8 = 0xA6;
pub(super) const DISABLE_PULLUPS: u8 = 0xAC;

// DMG commands
pub(super) const DMG_CART_READ: u8 = 0xB1;
const DMG_CART_WRITE: u8 = 0xB2;
pub(super) const DMG_CART_WRITE_SRAM: u8 = 0xB3;
pub(super) const DMG_MBC_RESET: u8 = 0xB4;

// Flash commands
pub(super) const SET_FLASH_CMD: u8 = 0xA7;
pub(super) const DMG_SET_BANK_CHANGE_CMD: u8 = 0xB8;
pub(super) const FLASH_PROGRAM: u8 = 0xD3;
const CART_WRITE_FLASH_CMD: u8 = 0xD4;

// Power commands
pub(super) const CART_PWR_ON: u8 = 0xF2;
pub(super) const CART_PWR_OFF: u8 = 0xF3;
pub(super) const QUERY_CART_PWR: u8 = 0xF4;

// Cleanup commands
pub(super) const SET_ADDR_AS_INPUTS: u8 = 0xA8;
pub(super) const OFW_DONE_LED_ON: u8 = 0x3D;

/// Send flash command sequence via CART_WRITE_FLASH_CMD (0xD4).
///
/// Each command is (address, value) where value is u16 (big-endian).
/// This uses the flash write pin rather than the normal cart bus.
pub(super) fn cart_write_flash(
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

// ── Protocol helpers ─────────────────────────────────────────────────

pub(super) fn write_cmd(port: &mut Box<dyn serialport::SerialPort>, data: &[u8]) -> Option<()> {
    port.write_all(data).ok()?;
    port.flush().ok()
}

pub(super) fn write_cmd_ack(
    port: &mut Box<dyn serialport::SerialPort>,
    data: &[u8],
    fw_ver: u16,
) -> Option<()> {
    write_cmd(port, data)?;
    if fw_ver >= 12 {
        match read_byte(port) {
            Some(0x01) | Some(0x03) => {}
            Some(_) => {
                return None;
            }
            None => {
                return None;
            }
        }
    }
    Some(())
}

/// DMG_CART_WRITE: [0xB2, addr(4B BE), value(1B)]
pub(super) fn cart_write(
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
pub(super) fn set_variable(
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

pub(super) fn read_byte(port: &mut Box<dyn serialport::SerialPort>) -> Option<u8> {
    let mut buf = [0u8; 1];
    port.read_exact(&mut buf).ok()?;
    Some(buf[0])
}
