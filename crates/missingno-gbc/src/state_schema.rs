//! The Game Boy Color hardware state schema: the DMG schema plus the colour
//! console's delta, composed the way the sidebar sections are — the shared Game
//! Boy fields with the CGB-only registers and engines folded into the hardware
//! each belongs to (KEY1 speed and SVBK banking in the CPU block; VBK, the
//! palette-index registers, and HDMA in the PPU/HDMA blocks; palette RAM in its
//! own CRAM region). This is DATA; `missingno-gbc` composes it from the public
//! DMG field builders in `missingno-gb`.
//!
//! CGB has no gate-level ground truth, so the delta's deep fields are named from
//! hardware register documentation, not a die concordance.

use std::sync::LazyLock;

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, SystemStateSchema,
};

use missingno_gb::frame::NATIVE_SIZE;
use missingno_gb::state_schema::{dmg_boundary_fields, dmg_memory_spans, dmg_observable_fields};

use FieldType::{Bool, U8, U16};

/// The CGB-only observable registers, absent on DMG.
fn cgb_observable_delta() -> Vec<FieldDef> {
    vec![
        FieldDef::observable("double_speed", Bool, "cpu").help("KEY1 bit 7 — CPU running at 2x"),
        FieldDef::observable("svbk", U8, "cpu").help("SVBK ($FF70) — work-RAM bank select"),
        FieldDef::observable("vbk", U8, "ppu").help("VBK ($FF4F) — VRAM bank select"),
        FieldDef::observable("opri", U8, "ppu").help("OPRI ($FF6C) — object priority mode"),
        FieldDef::observable("bcps", U8, "ppu").help("BCPS ($FF68) — background palette index"),
        FieldDef::observable("ocps", U8, "ppu").help("OCPS ($FF6A) — object palette index"),
    ]
}

/// The CGB-only deep state, absent on DMG: the VRAM-DMA (HDMA/GDMA) engine.
fn cgb_boundary_delta() -> Vec<FieldDef> {
    vec![
        FieldDef::boundary("hdma_active", Bool, "hdma").help("a VRAM DMA transfer is in progress"),
        FieldDef::boundary("hdma_source", U16, "hdma").help("HDMA1/2 — next source address"),
        FieldDef::boundary("hdma_dest", U16, "hdma").help("HDMA3/4 — next VRAM destination"),
        FieldDef::boundary("hdma_remaining", U8, "hdma").help("16-byte blocks left to transfer"),
        FieldDef::boundary("hdma_hblank", Bool, "hdma")
            .help("HBlank-paced mode (HDMA) rather than a single burst (GDMA)"),
    ]
}

/// The CGB memory spans that diverge from the DMG's: bank-complete VRAM (both
/// 8 KiB banks) and WRAM (all eight 4 KiB banks) as linear off-bus images, and
/// the palette RAM reached through the index ports rather than the CPU map.
fn cgb_memory_delta() -> Vec<MemorySpan> {
    vec![
        MemorySpan::off_bus("vram", 2 * 0x2000).help("video RAM, both banks (bank 0 then bank 1)"),
        MemorySpan::off_bus("wram", 8 * 0x1000).help("work RAM, all eight banks (bank order)"),
        MemorySpan::off_bus("cram_bg", 64).help("background palette RAM (8 palettes × 4 × RGB555)"),
        MemorySpan::off_bus("cram_obj", 64).help("object palette RAM (8 palettes × 4 × RGB555)"),
    ]
}

static CGB_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| {
    let mut fields = dmg_observable_fields();
    fields.extend(cgb_observable_delta());
    fields.extend(dmg_boundary_fields());
    fields.extend(cgb_boundary_delta());

    // The colour delta replaces the DMG's single-bank VRAM/WRAM spans with
    // bank-complete images; keep the DMG's OAM / wave RAM / high RAM / cart RAM.
    let mut memory: Vec<MemorySpan> = dmg_memory_spans()
        .into_iter()
        .filter(|span| span.name != "vram" && span.name != "wram")
        .collect();
    memory.extend(cgb_memory_delta());

    SystemStateSchema {
        system: "cgb",
        fields,
        memory,
        frame: FrameSpec {
            width: NATIVE_SIZE.0,
            height: Some(NATIVE_SIZE.1),
            format: PixelFormat::Rgb555,
        },
    }
});

/// The CGB hardware state schema.
pub fn cgb_state_schema() -> &'static SystemStateSchema {
    &CGB_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::state::PixelFormat;

    #[test]
    fn schema_is_well_formed() {
        assert_eq!(cgb_state_schema().check(), Ok(()));
    }

    #[test]
    fn cgb_is_dmg_plus_the_delta() {
        let cgb = cgb_state_schema();
        // Every DMG field is present.
        for dmg in dmg_observable_fields()
            .iter()
            .chain(dmg_boundary_fields().iter())
        {
            assert!(
                cgb.field(dmg.name).is_some(),
                "CGB schema is missing DMG field '{}'",
                dmg.name
            );
        }
        // The colour delta is present.
        for name in [
            "double_speed",
            "svbk",
            "vbk",
            "opri",
            "bcps",
            "ocps",
            "hdma_active",
            "hdma_source",
            "hdma_dest",
            "hdma_remaining",
            "hdma_hblank",
        ] {
            assert!(cgb.field(name).is_some(), "CGB schema is missing '{name}'");
        }
        assert!(cgb.span("cram_bg").is_some());
        assert!(cgb.span("cram_obj").is_some());
    }

    #[test]
    fn cgb_memory_spans_are_bank_complete() {
        let cgb = cgb_state_schema();
        // Both VRAM banks (16 KiB) and all eight WRAM banks (32 KiB), off-bus.
        let vram = cgb.span("vram").expect("vram span");
        assert_eq!(vram.len, 0x4000);
        assert_eq!(vram.start, None);
        let wram = cgb.span("wram").expect("wram span");
        assert_eq!(wram.len, 0x8000);
        assert_eq!(wram.start, None);
        // Palette RAM: 64 bytes each.
        assert_eq!(cgb.span("cram_bg").unwrap().len, 64);
        assert_eq!(cgb.span("cram_obj").unwrap().len, 64);
    }

    #[test]
    fn cgb_identity_and_frame() {
        let cgb = cgb_state_schema();
        assert_eq!(cgb.system, "cgb");
        assert_eq!(cgb.frame.format, PixelFormat::Rgb555);
    }
}
