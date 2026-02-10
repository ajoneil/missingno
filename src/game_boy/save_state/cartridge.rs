use nanoserde::{DeRon, SerRon};

use super::base64::Base64Bytes;

#[derive(SerRon, DeRon)]
pub enum MbcState {
    NoMbc {
        ram: Option<Base64Bytes>,
    },
    Mbc1 {
        ram: Option<Base64Bytes>,
        advanced: bool,
        ram_enabled: bool,
        bank: u8,
        ram_bank: u8,
    },
    Mbc2 {
        ram: Base64Bytes,
        ram_enabled: bool,
        bank: u8,
    },
    Mbc3 {
        ram: Base64Bytes,
        ram_and_clock_enabled: bool,
        bank: u8,
        mapped: Mbc3MappedState,
        clock: Option<Mbc3ClockState>,
    },
    Mbc5 {
        ram: Base64Bytes,
        ram_enabled: bool,
        rom_bank: u16,
        ram_bank: u8,
    },
    Mbc6 {
        ram: Base64Bytes,
        flash: Base64Bytes,
        ram_enabled: bool,
        flash_enabled: bool,
        rom_bank_a: u8,
        rom_bank_a_flash: bool,
        rom_bank_b: u8,
        rom_bank_b_flash: bool,
        ram_bank_a: u8,
        ram_bank_b: u8,
    },
    Mbc7 {
        eeprom_data: Base64Bytes,
        eeprom_write_enabled: bool,
        ram_enabled_1: bool,
        ram_enabled_2: bool,
        rom_bank: u8,
        accel_x: u16,
        accel_y: u16,
    },
    Huc1 {
        ram: Base64Bytes,
        rom_bank: u8,
        ram_bank: u8,
        ir_mode: bool,
    },
    Huc3 {
        ram: Base64Bytes,
        rom_bank: u8,
        ram_bank: u8,
        mode: u8,
        rtc_memory: Base64Bytes,
        rtc_address: u8,
        rtc_last_command: u8,
        rtc_response: u8,
        rtc_semaphore: u8,
    },
}

#[derive(SerRon, DeRon)]
pub enum Mbc3MappedState {
    Ram(u8),
    Clock(u8),
}

#[derive(SerRon, DeRon)]
pub struct Mbc3ClockState {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub days_lower: u8,
    pub days_upper: u8,
    pub latched_seconds: u8,
    pub latched_minutes: u8,
    pub latched_hours: u8,
    pub latched_days_lower: u8,
    pub latched_days_upper: u8,
    pub latch_ready: bool,
}
