//! The SG-1000's hardware state schema — the authored, hardware-named
//! description of machine state a save state keys its record on. This is DATA,
//! not capture logic; `snapshot.rs` fills it and reads it back.
//!
//! Tier 1 is the programmer-observable surface: the Z80 register file, the
//! VDP's register file, pointer and status flags, the PSG's register file, and
//! the two joystick multiplexer bytes. Tier 2a is what a bit-exact restore also
//! needs — the CPU's boundary carries (MEMPTR, the Q latch's source, the
//! interrupt latches and the sampled /INT level), the VDP's raster counters,
//! port engine, status set instants, sprite-scanner lattice position and
//! latched fetch, and the PSG's counters, flip-flops and shift register.
//!
//! A save is taken at an instruction boundary, where the Z80's sequencer is
//! absent, so there is no Tier-2b residue to name. The VDP's own quantum is
//! finer than the CPU's, and it is captured whole — its counters and in-flight
//! access are live wherever the boundary falls.
//!
//! Excluded by design: the recorded bus trace (a diagnostic of the instruction
//! just run) and the accumulated audio samples (drained output, not state). No
//! SG-1000 board switches banks, so a cartridge carries nothing but whatever
//! RAM it holds.

use std::sync::LazyLock;

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, SystemStateSchema,
};
use missingno_ti_vdp::VISIBLE_WIDTH;

use FieldType::{U8, U32};

/// The work RAM's own kilobyte, at the base of the window `/CS WRAM` selects.
const RAM_BASE: u32 = 0xC000;
const RAM_SIZE: u32 = 0x400;

/// The board's own fields: the joystick lines, and the output tap and raster
/// handoff it keeps beside the chips.
fn board_fields() -> Vec<FieldDef> {
    vec![
        FieldDef::observable("joystick_dc", U8, "board")
            .help("the $DC multiplexer byte — active low"),
        FieldDef::observable("joystick_dd", U8, "board")
            .help("the $DD multiplexer byte — active low"),
        FieldDef::boundary("audio_sample_phase", U32, "board")
            .help("the 44.1 kHz output tap's carried phase, in T-states — an output stage, not board silicon")
            .sourced("missingno")
            .nullable(),
        FieldDef::boundary("fields_taken", U32, "board")
            .help("rasters the board has handed out, so a completed one is not handed out twice")
            .sourced("missingno"),
    ]
}

/// The three chips' fields and the board's, the observable surface of each
/// ahead of any boundary carry. The pause switch is what drives this board's
/// /NMI.
fn fields() -> Vec<FieldDef> {
    let mut fields = missingno_zilog_z80::record::state_fields();
    fields.extend(missingno_ti_vdp::record::state_fields());
    fields.extend(missingno_ti_psg::record::state_fields());
    fields.extend(board_fields());
    fields.sort_by_key(|field| field.tier);
    for field in &mut fields {
        if field.name == "nmi_pending" {
            field.help = Some("the pause switch pulled /NMI down");
        }
    }
    fields
}

/// The byte regions a save state carries: the work RAM, whatever RAM the
/// cartridge holds, the VDP's DRAM, and the two line buffers under the raster.
/// The field being emitted travels as the state's framebuffer, since a mid-field
/// save cannot reconstruct the rows already put down.
fn memory_spans() -> Vec<MemorySpan> {
    let mut spans = vec![
        MemorySpan::addressable("work_ram", RAM_BASE, RAM_SIZE)
            .help("the TMM2009's kilobyte, before the decode mirrors it"),
        // Where a board's RAM answers and how much of it there is are the
        // board's own decode, so it travels as one linear region.
        MemorySpan::off_bus("cart_ram", 0)
            .optional()
            .help("the cartridge's RAM chips, in the order the board decodes them"),
    ];
    spans.extend(missingno_ti_vdp::record::memory_spans());
    spans
}

/// The picture the console hands out: the VDP's visible raster — the display
/// area inside its backdrop border — as TI colour indices.
fn frame() -> FrameSpec {
    FrameSpec {
        width: VISIBLE_WIDTH as u32,
        height: None,
        format: PixelFormat::Indexed8,
    }
}

static SG1000_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| SystemStateSchema {
    system: "sg1000",
    isa: "z80",
    instruction_addr_field: "pc",
    entry: None,
    fields: fields(),
    memory: memory_spans(),
    frame: frame(),
});

/// The Sega SG-1000 hardware state schema.
pub fn sg1000_state_schema() -> &'static SystemStateSchema {
    &SG1000_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::state::{Provenance, Tier};

    #[test]
    fn schema_is_well_formed() {
        assert_eq!(sg1000_state_schema().check(), Ok(()));
    }

    #[test]
    fn every_field_names_a_subsystem_of_the_board() {
        for field in &sg1000_state_schema().fields {
            assert!(!field.name.is_empty());
            assert!(
                ["cpu", "vdp", "psg", "board"].contains(&field.subsystem),
                "{} names {}",
                field.name,
                field.subsystem
            );
        }
    }

    #[test]
    fn carries_the_register_files_and_the_two_memories() {
        let schema = sg1000_state_schema();
        for name in ["a", "f", "pc", "sp", "ix", "iy", "i", "r"] {
            assert!(schema.field(name).is_some(), "missing {name}");
        }
        for name in ["vdp_r0", "vdp_r7", "vdp_address", "psg_tone1_period"] {
            assert!(schema.field(name).is_some(), "missing {name}");
        }
        assert_eq!(schema.span("work_ram").map(|span| span.len), Some(0x400));
        assert_eq!(schema.span("vram").map(|span| span.len), Some(0x4000));
    }

    /// Only the two bookkeeping counters and the output tap are emulator
    /// probes; everything else names hardware.
    #[test]
    fn the_emulator_probes_are_named_as_such() {
        let probes: Vec<&str> = sg1000_state_schema()
            .fields
            .iter()
            .filter(|field| matches!(field.provenance, Provenance::Emulator(_)))
            .map(|field| field.name)
            .collect();
        assert_eq!(
            probes,
            ["vdp_fields_completed", "audio_sample_phase", "fields_taken"]
        );
    }

    #[test]
    fn the_cpu_register_file_is_observable_and_its_carries_are_not() {
        let schema = sg1000_state_schema();
        assert_eq!(schema.field("pc").unwrap().tier, Tier::Observable);
        assert_eq!(schema.field("wz").unwrap().tier, Tier::Boundary);
    }
}
