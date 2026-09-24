//! The ColecoVision's hardware state schema — the authored, hardware-named
//! description of machine state a save state keys its record on. This is DATA,
//! not capture logic; `snapshot.rs` fills it and reads it back.
//!
//! The three chips state their own fields; the board adds the controller mode
//! latch, the /M1 wait latch and the switches each hand controller holds. The
//! BIOS is firmware, not state: a restore runs against whichever image the
//! console was built with.

use std::sync::LazyLock;

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, SystemStateSchema,
};
use missingno_ti_vdp::VISIBLE_WIDTH;

use FieldType::{Bool, U8, U16, U32};

/// Where the 2114 pair's kilobyte first answers, in the window U5's Y3 selects.
const RAM_BASE: u32 = 0x6000;
const RAM_SIZE: u32 = 0x400;

fn board_fields() -> Vec<FieldDef> {
    vec![
        FieldDef::observable("controller_mode", Bool, "board")
            .help("the U24 mode latch — set by a $80-$9F write (keypad segment), reset by $C0-$DF (joystick)"),
        FieldDef::boundary("wait_latch_q", Bool, "board")
            .help("U8A's Q — the /M1 wait flip-flop pulling /WAIT low"),
        FieldDef::observable("pad_lines_1", U8, "board")
            .help("J5's stick lines, bits 0-3 Up/Right/Down/Left — active low"),
        FieldDef::observable("pad_buttons_1", U8, "board")
            .help("J5's buttons held: bit 0 left fire, bit 1 right fire"),
        FieldDef::observable("keys_1", U16, "board")
            .help("J5's keypad keys held, bit n for 1-9, *, 0, #"),
        FieldDef::observable("pad_lines_2", U8, "board")
            .help("J6's stick lines, bits 0-3 Up/Right/Down/Left — active low"),
        FieldDef::observable("pad_buttons_2", U8, "board")
            .help("J6's buttons held: bit 0 left fire, bit 1 right fire"),
        FieldDef::observable("keys_2", U16, "board")
            .help("J6's keypad keys held, bit n for 1-9, *, 0, #"),
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
/// ahead of any boundary carry. The VDP's /INT is what drives this board's
/// /NMI.
fn fields() -> Vec<FieldDef> {
    let mut fields = missingno_zilog_z80::record::state_fields();
    fields.extend(missingno_ti_vdp::record::state_fields());
    fields.extend(missingno_ti_psg::record::state_fields());
    fields.extend(board_fields());
    fields.sort_by_key(|field| field.tier);
    for field in &mut fields {
        match field.name {
            "nmi_pending" => {
                field.help = Some("a falling edge of the VDP's /INT awaits acceptance")
            }
            "nmi_line" => field.help = Some("the VDP's /INT as the /NMI pin last saw it"),
            _ => {}
        }
    }
    fields
}

/// The byte regions a save state carries: the RAM, the VDP's DRAM, and the two
/// line buffers under the raster.
fn memory_spans() -> Vec<MemorySpan> {
    let mut spans = vec![
        MemorySpan::addressable("ram", RAM_BASE, RAM_SIZE)
            .help("the 2114 pair's kilobyte, before the decode mirrors it"),
    ];
    spans.extend(missingno_ti_vdp::record::memory_spans());
    spans
}

/// The picture the console hands out: the VDP's visible raster as TI colour
/// indices.
fn frame() -> FrameSpec {
    FrameSpec {
        width: VISIBLE_WIDTH as u32,
        height: None,
        format: PixelFormat::Indexed8,
    }
}

static COLECOVISION_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| SystemStateSchema {
    system: "colecovision",
    isa: "z80",
    instruction_addr_field: "pc",
    entry: None,
    fields: fields(),
    memory: memory_spans(),
    frame: frame(),
});

/// The ColecoVision hardware state schema.
pub fn colecovision_state_schema() -> &'static SystemStateSchema {
    &COLECOVISION_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::state::{Provenance, Tier};

    #[test]
    fn schema_is_well_formed() {
        assert_eq!(colecovision_state_schema().check(), Ok(()));
    }

    #[test]
    fn every_field_names_a_subsystem_of_the_board() {
        for field in &colecovision_state_schema().fields {
            assert!(
                ["cpu", "vdp", "psg", "board"].contains(&field.subsystem),
                "{} names {}",
                field.name,
                field.subsystem
            );
        }
    }

    #[test]
    fn carries_the_register_files_the_latches_and_the_two_memories() {
        let schema = colecovision_state_schema();
        for name in [
            "a",
            "pc",
            "nmi_line",
            "vdp_r1",
            "psg_tone1_period",
            "controller_mode",
            "wait_latch_q",
            "keys_2",
        ] {
            assert!(schema.field(name).is_some(), "missing {name}");
        }
        let ram = schema.span("ram").expect("the RAM span");
        assert_eq!((ram.start, ram.len), (Some(0x6000), 0x400));
        assert_eq!(schema.span("vram").map(|span| span.len), Some(0x4000));
    }

    #[test]
    fn the_emulator_probes_are_named_as_such() {
        let probes: Vec<&str> = colecovision_state_schema()
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
    fn the_latches_are_tiered_by_what_a_program_can_see() {
        let schema = colecovision_state_schema();
        assert_eq!(
            schema.field("controller_mode").unwrap().tier,
            Tier::Observable
        );
        assert_eq!(schema.field("wait_latch_q").unwrap().tier, Tier::Boundary);
    }
}
