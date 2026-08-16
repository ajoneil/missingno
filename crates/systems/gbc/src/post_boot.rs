//! The state the CGB boot ROM hands off, and the reset that precedes it.

use missingno_gb::audio::Audio;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::cpu::Cpu;
use missingno_gb::cpu::flags::Flags;
use missingno_gb::dma::Dma;
use missingno_gb::joypad::{Buttons, Joypad};
use missingno_gb::ppu::Ppu;
use missingno_gb::timers::Timers;

use crate::{Cgb, CgbApu, CgbPpu};

pub(crate) fn cpu(_checksum: u8) -> Cpu {
    // CPU-CGB-C post-boot register file. A=$11 signals CGB hardware to the
    // cartridge; unlike DMG, the flags don't depend on the header checksum.
    Cpu::post_boot_with(0x11, 0x00, 0x00, 0x00, 0x08, 0x00, 0x7c, Flags::ZERO)
}

/// CGB boot-ROM handoff divider phase. The boot ROM runs longer for a
/// DMG cartridge (compat-palette setup): FF04 reads $1E / $26.
pub(crate) fn timers(cgb_cart: bool) -> Timers {
    Timers::post_boot_with_counter(if cgb_cart { 0x47A8 } else { 0x099F })
}

/// The CGB boot ROM hands the APU off one frame-sequencer step earlier than
/// the DMG boot ROM (measured at PC=$0100). DMG-compat carts run a different
/// boot sequence whose phase is unmeasured, so they keep the DMG handoff.
pub(crate) fn audio(internal_counter: u16, cgb_cart: bool) -> Audio<CgbApu> {
    if cgb_cart {
        let mut audio = Audio::post_boot_with_fs_step(internal_counter, 1);
        // The CGB boot chime leaves CH1 at this duty/divider phase, distinct
        // from the DMG handoff the `Default` channel state encodes.
        audio.set_ch1_post_boot_phase(6, 0x7DA);
        audio
    } else {
        Audio::post_boot(internal_counter)
    }
}

/// CGB boot-ROM handoff is mid-VBlank; the line depends on the boot
/// duration (CGB cart: line 144, dot ~164; DMG cart: line 148, dot ~356).
/// The boot ROM also zeroes OBP0/OBP1 (DMG leaves them at $FF).
pub(crate) fn ppu(cgb_cart: bool) -> Ppu<CgbPpu> {
    let mut ppu = if cgb_cart {
        Ppu::post_boot_vblank_handoff(144, 41)
    } else {
        Ppu::post_boot_vblank_handoff(148, 88)
    };
    ppu.set_post_boot_object_palettes(0x00);
    ppu
}

/// The CGB boot ROM hands off with both key-matrix lines deselected
/// (P1 reads $FF).
pub(crate) fn joypad() -> Joypad {
    Joypad {
        read_buttons: false,
        read_dpad: false,
        pressed: Buttons::empty(),
    }
}

/// The CGB boot ROM leaves FF46 reading $00.
pub(crate) fn dma() -> Dma {
    Dma::with_source_register(0x00)
}

impl Cgb {
    pub(crate) fn reset_for_cartridge(&mut self, cartridge: &Cartridge, has_boot_rom: bool) {
        *self = Self::default();
        // A DMG cartridge boots the CGB into compatibility mode (KEY0 bit 2).
        // With a real boot ROM that decision is the boot ROM's (via KEY0);
        // only HLE it on the skip-boot path.
        if !has_boot_rom {
            self.dmg_compat = !cartridge.is_cgb();
        }
    }
}
