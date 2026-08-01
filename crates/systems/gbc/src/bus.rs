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
