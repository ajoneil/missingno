//! Battery-save serialization: raw SRAM, with the BGB/VBA-M real-time-clock
//! tail appended on RTC carts so the clock survives across sessions and the
//! saves stay interchangeable with other emulators.
//!
//! Tail layout: five u32 LE current registers (seconds, minutes, hours, day
//! low, day high), five u32 LE latched registers, then the save moment as a
//! u64 LE unix timestamp (48 bytes) — the older 44-byte u32-timestamp variant
//! is accepted on load.

use missingno_gb::cartridge::{Cartridge, RtcSnapshot};

/// SRAM sizes are KiB multiples, so a tail is recognisable from the length.
fn tail_length(len: usize) -> Option<usize> {
    match len % 1024 {
        48 => Some(48),
        44 => Some(44),
        _ => None,
    }
}

/// The battery-backed contents to persist: SRAM plus the RTC tail when the
/// cart has a clock. `None` when there's nothing to save.
pub fn save_blob(cartridge: &Cartridge, now_unix: u64) -> Option<Vec<u8>> {
    let mut blob = cartridge.ram()?;
    if let Some(rtc) = cartridge.rtc() {
        for register in registers(&rtc) {
            blob.extend((register as u32).to_le_bytes());
        }
        blob.extend(now_unix.to_le_bytes());
    }
    Some(blob)
}

/// Split a loaded save into SRAM and any RTC tail (with its save moment).
pub fn split_blob(blob: Vec<u8>) -> (Vec<u8>, Option<(RtcSnapshot, u64)>) {
    let Some(tail_length) = tail_length(blob.len()) else {
        return (blob, None);
    };
    let (ram, tail) = blob.split_at(blob.len() - tail_length);

    let register =
        |index: usize| u32::from_le_bytes(tail[index * 4..index * 4 + 4].try_into().unwrap()) as u8;
    let mut values = [0u8; 10];
    for (index, value) in values.iter_mut().enumerate() {
        *value = register(index);
    }
    let snapshot = snapshot_from(values);

    let saved_at = if tail_length == 48 {
        u64::from_le_bytes(tail[40..48].try_into().unwrap())
    } else {
        u32::from_le_bytes(tail[40..44].try_into().unwrap()) as u64
    };
    (ram.to_vec(), Some((snapshot, saved_at)))
}

fn registers(rtc: &RtcSnapshot) -> [u8; 10] {
    [
        rtc.registers.seconds,
        rtc.registers.minutes,
        rtc.registers.hours,
        rtc.registers.days_lower,
        rtc.registers.days_upper,
        rtc.latched.seconds,
        rtc.latched.minutes,
        rtc.latched.hours,
        rtc.latched.days_lower,
        rtc.latched.days_upper,
    ]
}

fn snapshot_from(values: [u8; 10]) -> RtcSnapshot {
    use missingno_gb::cartridge::mbc::mbc3::ClockRegisters;
    RtcSnapshot {
        registers: ClockRegisters {
            seconds: values[0],
            minutes: values[1],
            hours: values[2],
            days_lower: values[3],
            days_upper: values[4],
        },
        latched: ClockRegisters {
            seconds: values[5],
            minutes: values[6],
            hours: values[7],
            days_lower: values[8],
            days_upper: values[9],
        },
    }
}

pub fn now_unix() -> u64 {
    jiff::Timestamp::now().as_second().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_saves_pass_through_untouched() {
        let blob = vec![0xaa; 8 * 1024];
        let (ram, rtc) = split_blob(blob.clone());
        assert_eq!(ram, blob);
        assert!(rtc.is_none());
    }

    #[test]
    fn rtc_tail_round_trips() {
        let mut blob = vec![0x55; 8 * 1024];
        let registers = [12u8, 34, 5, 200, 1, 11, 33, 4, 199, 0];
        for &value in &registers {
            blob.extend((value as u32).to_le_bytes());
        }
        blob.extend(1_700_000_000u64.to_le_bytes());

        let (ram, rtc) = split_blob(blob);
        assert_eq!(ram.len(), 8 * 1024);
        let (snapshot, saved_at) = rtc.unwrap();
        assert_eq!(snapshot.registers.seconds, 12);
        assert_eq!(snapshot.registers.days_upper, 1);
        assert_eq!(snapshot.latched.days_lower, 199);
        assert_eq!(saved_at, 1_700_000_000);
    }

    #[test]
    fn short_timestamp_variant_is_accepted() {
        let mut blob = vec![0u8; 2 * 1024];
        for value in 0..10u32 {
            blob.extend(value.to_le_bytes());
        }
        blob.extend(1_600_000_000u32.to_le_bytes());
        let (ram, rtc) = split_blob(blob);
        assert_eq!(ram.len(), 2 * 1024);
        assert_eq!(rtc.unwrap().1, 1_600_000_000);
    }
}
