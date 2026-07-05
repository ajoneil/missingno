mod detect;
mod flash;
mod protocol;
mod transfer;

pub use detect::{CartridgeHeader, DetectedDevice, detect_ports, list_ports};
pub use flash::{FlashPhase, FlashProgress, flash_rom};
pub use transfer::{DumpProgress, dump_rom, read_sram, write_sram};

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
