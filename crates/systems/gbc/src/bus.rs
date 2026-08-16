//! The CGB bus split and the OAM-DMA conflicts it decides.

use missingno_gb::shared_oam_dma_write_conflict_byte;

/// CGB splits the cartridge and WRAM onto separate buses (DMG shares one
/// external bus), so the CPU can touch one while OAM DMA drives the other.
#[derive(PartialEq)]
pub(crate) enum CgbBus {
    Cartridge,
    WorkRam,
    Video,
}

pub(crate) fn cgb_bus(address: u16) -> Option<CgbBus> {
    match address {
        0x8000..=0x9FFF => Some(CgbBus::Video),
        0xC000..=0xFDFF => Some(CgbBus::WorkRam),
        0x0000..=0x7FFF | 0xA000..=0xBFFF => Some(CgbBus::Cartridge),
        _ => None,
    }
}

/// The bus an OAM-DMA *source* page drives, per the DMA decoder's external-RAM
/// `/CS` for `$A0–$FF`. Differs from `cgb_bus` in the echo region: `$E000–$FDFF`
/// is WRAM to the CPU but, to the DMA, is past the cart-RAM window — the
/// cartridge bus (which floats to `$FF`, see `dma_source_open_bus`). `$C0–$DF`
/// still reaches real WRAM on the WRAM bus.
pub(crate) fn cgb_dma_source_bus(address: u16) -> CgbBus {
    match address {
        0x8000..=0x9FFF => CgbBus::Video,
        0xC000..=0xDFFF => CgbBus::WorkRam,
        _ => CgbBus::Cartridge,
    }
}

pub(crate) fn oam_dma_bus_conflict(cpu_addr: u16, dma_source: u16) -> bool {
    cgb_bus(cpu_addr) == Some(cgb_dma_source_bus(dma_source))
}

/// A WRAM-bus access taken while the DMA sources from the cart bus has its
/// `$C000`/`$D000` half-selector (A12) driven by the DMA source page; the low
/// 12 bits stay the CPU's. A VRAM or WRAM source leaves the access untouched.
pub(crate) fn oam_dma_wram_remap(cpu_addr: u16, dma_source: u16) -> Option<u16> {
    (cgb_bus(cpu_addr) == Some(CgbBus::WorkRam)
        && cgb_dma_source_bus(dma_source) == CgbBus::Cartridge)
        .then_some((dma_source & 0x1000) | (cpu_addr & 0x0FFF) | 0xC000)
}

/// On the WRAM bus the colliding CPU write sits on a different bus from the
/// DMA source, so it never reaches the OAM write phase — the DMA deposits the
/// raw byte it fetched. Other source buses follow the shared model.
pub(crate) fn oam_dma_write_conflict_byte(src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
    if cgb_dma_source_bus(dma_source) == CgbBus::WorkRam {
        src_byte
    } else {
        shared_oam_dma_write_conflict_byte(src_byte, cpu_value, dma_source)
    }
}

pub(crate) fn oam_dma_conflict_zeroes_oam(cpu_addr: u16, dma_source: u16) -> bool {
    cgb_dma_source_bus(dma_source) == CgbBus::Video && cgb_bus(cpu_addr) == Some(CgbBus::Video)
}

/// VBK re-banks a VRAM-source DMA, SVBK a WRAM-source DMA; the matching write
/// latches one byte late so the coincident DMA byte reads the prior bank.
pub(crate) fn oam_dma_source_bank_write(address: u16, dma_source: u16) -> bool {
    match address {
        0xFF4F => cgb_dma_source_bus(dma_source) == CgbBus::Video,
        0xFF70 => cgb_dma_source_bus(dma_source) == CgbBus::WorkRam,
        _ => false,
    }
}

pub(crate) fn dma_source_open_bus(address: u16) -> Option<u8> {
    (address >= 0xE000).then_some(0xFF)
}
