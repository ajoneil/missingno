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
use missingno_ti_vdp::{VISIBLE_WIDTH, VRAM_SIZE};

use crate::console::STANDARD;

use FieldType::{Bool, U8, U16, U32};

/// The work RAM's own kilobyte, at the base of the window `/CS WRAM` selects.
const RAM_BASE: u32 = 0xC000;
const RAM_SIZE: u32 = 0x400;
/// The line-latched sprite plane covers the display area only.
const SPRITE_PLANE_WIDTH: u32 = 256;

/// Tier-1 observable fields: the three register files and the joystick lines.
fn observable_fields() -> Vec<FieldDef> {
    let mut fields = vec![
        FieldDef::observable("a", U8, "cpu").help("accumulator"),
        FieldDef::observable("f", U8, "cpu").help("flags"),
        FieldDef::observable("b", U8, "cpu"),
        FieldDef::observable("c", U8, "cpu"),
        FieldDef::observable("d", U8, "cpu"),
        FieldDef::observable("e", U8, "cpu"),
        FieldDef::observable("h", U8, "cpu"),
        FieldDef::observable("l", U8, "cpu"),
        FieldDef::observable("a_alt", U8, "cpu").help("A' — the alternate set"),
        FieldDef::observable("f_alt", U8, "cpu").help("F'"),
        FieldDef::observable("b_alt", U8, "cpu").help("B'"),
        FieldDef::observable("c_alt", U8, "cpu").help("C'"),
        FieldDef::observable("d_alt", U8, "cpu").help("D'"),
        FieldDef::observable("e_alt", U8, "cpu").help("E'"),
        FieldDef::observable("h_alt", U8, "cpu").help("H'"),
        FieldDef::observable("l_alt", U8, "cpu").help("L'"),
        FieldDef::observable("ix", U16, "cpu"),
        FieldDef::observable("iy", U16, "cpu"),
        FieldDef::observable("sp", U16, "cpu").help("stack pointer"),
        FieldDef::observable("pc", U16, "cpu").help("program counter"),
        FieldDef::observable("i", U8, "cpu").help("interrupt vector page"),
        FieldDef::observable("r", U8, "cpu").help("memory refresh counter"),
        FieldDef::observable("iff1", Bool, "cpu").help("interrupts enabled"),
        FieldDef::observable("iff2", Bool, "cpu").help("IFF1's copy, which LD A,I reads"),
        FieldDef::observable("im", U8, "cpu").help("interrupt mode 0/1/2"),
        FieldDef::observable("halted", Bool, "cpu").help("HALT is refetching"),
    ];

    for (index, register) in [
        "vdp_r0", "vdp_r1", "vdp_r2", "vdp_r3", "vdp_r4", "vdp_r5", "vdp_r6", "vdp_r7",
    ]
    .into_iter()
    .enumerate()
    {
        fields.push(FieldDef::observable(register, U8, "vdp").help(match index {
            0 => "mode bits M3 and the external-video enable",
            1 => "RAM size, display and interrupt enables, M1/M2, sprite size and MAG",
            2 => "name table base",
            3 => "colour table base",
            4 => "pattern generator base",
            5 => "sprite attribute table base",
            6 => "sprite pattern generator base",
            _ => "text colour and backdrop",
        }));
    }
    fields.push(
        FieldDef::observable("vdp_address", U16, "vdp").help("the auto-incrementing VRAM pointer"),
    );
    fields.push(FieldDef::observable("vdp_frame_flag", Bool, "vdp").help("status F"));
    fields.push(FieldDef::observable("vdp_fifth_sprite_flag", Bool, "vdp").help("status 5S"));
    fields.push(FieldDef::observable("vdp_coincidence_flag", Bool, "vdp").help("status C"));
    fields.push(
        FieldDef::observable("vdp_fifth_sprite_index", U8, "vdp")
            .help("the SAT index latched with 5S"),
    );

    for (period, attenuation) in [
        ("psg_tone1_period", "psg_tone1_attenuation"),
        ("psg_tone2_period", "psg_tone2_attenuation"),
        ("psg_tone3_period", "psg_tone3_attenuation"),
    ] {
        fields.push(FieldDef::observable(period, U16, "psg").help("10-bit period register"));
        fields
            .push(FieldDef::observable(attenuation, U8, "psg").help("4-bit attenuation register"));
    }
    fields.push(
        FieldDef::observable("psg_noise_attenuation", U8, "psg").help("4-bit attenuation register"),
    );
    fields.push(
        FieldDef::observable("psg_noise_control", U8, "psg")
            .help("noise register: feedback in bit 2, shift rate in bits 1-0"),
    );
    fields.push(
        FieldDef::observable("psg_latched_register", U8, "psg").help(
            "the register address held between transfers (channel in bits 2-1, type in bit 0)",
        ),
    );

    fields.push(
        FieldDef::observable("joystick_dc", U8, "board")
            .help("the $DC multiplexer byte — active low"),
    );
    fields.push(
        FieldDef::observable("joystick_dd", U8, "board")
            .help("the $DD multiplexer byte — active low"),
    );

    fields
}

/// Tier-2a boundary-complete deep state: the CPU's boundary carries, the VDP's
/// counters and engines, and the PSG's generators.
fn boundary_fields() -> Vec<FieldDef> {
    let mut fields = vec![
        FieldDef::boundary("wz", U16, "cpu").help("MEMPTR"),
        FieldDef::boundary("q", U8, "cpu").help("F left by the last flag-modifying instruction"),
        FieldDef::boundary("p", Bool, "cpu").help("LD A,I / LD A,R just took PF from IFF2"),
        FieldDef::boundary("ei_pending", Bool, "cpu")
            .help("acceptance is held off for the instruction after EI"),
        FieldDef::boundary("flags_touched", Bool, "cpu")
            .help("the retiring instruction wrote flags, so Q takes F"),
        FieldDef::boundary("nmi_pending", Bool, "cpu").help("the pause switch pulled /NMI down"),
        FieldDef::boundary("irq_line", Bool, "cpu").help("/INT as the board drives it"),
        FieldDef::boundary("irq_sampled", Bool, "cpu")
            .help("/INT as sampled at the last instruction's final T-state"),
        FieldDef::boundary("address_bus", U16, "cpu")
            .help("the address the pins hold through an internal T-state"),
    ];

    fields.extend([
        FieldDef::boundary("vdp_line", U16, "vdp").help("the vertical counter"),
        FieldDef::boundary("vdp_line_xtal", U16, "vdp")
            .help("XTAL periods elapsed within the line"),
        FieldDef::boundary("vdp_fields_completed", U32, "vdp")
            .help("visible rasters completed since power-on")
            .sourced("missingno"),
        FieldDef::boundary("vdp_awaiting_second_byte", Bool, "vdp")
            .help("the control port holds a first byte"),
        FieldDef::boundary("vdp_read_buffer", U8, "vdp").help("the read-ahead buffer"),
        FieldDef::boundary("vdp_transfer_write", Bool, "vdp")
            .help("the latched transfer is a write, not a read-ahead refill"),
        FieldDef::boundary("vdp_transfer_data", U8, "vdp").help("the latched transfer's byte"),
        FieldDef::boundary("vdp_prior_transfer_write", Bool, "vdp")
            .help("the transfer this one replaced"),
        FieldDef::boundary("vdp_prior_transfer_data", U8, "vdp"),
        FieldDef::boundary("vdp_transfer_written_ago", U32, "vdp")
            .help("XTAL periods since the transfer register was written"),
        FieldDef::boundary("vdp_pending_address", U16, "vdp")
            .help("the address latched by the request that raised the flag"),
        FieldDef::boundary("vdp_pending_flag", Bool, "vdp").help("a CPU access is waiting"),
        FieldDef::boundary("vdp_access_active", Bool, "vdp")
            .help("a memory cycle has claimed the access"),
        FieldDef::boundary("vdp_access_address", U16, "vdp"),
        FieldDef::boundary("vdp_access_claimed_ago", U8, "vdp")
            .help("XTAL periods since the claim; the lock and release follow"),
        FieldDef::boundary("vdp_frame_flag_set_ago", U32, "vdp")
            .help("XTAL periods since F was set, which a read's strobe races"),
        FieldDef::boundary("vdp_fifth_sprite_set_ago", U32, "vdp")
            .help("XTAL periods since 5S was set"),
        FieldDef::boundary("vdp_scan_counter", U8, "vdp")
            .help("the sprite pre-processing scanner's SAT index"),
        FieldDef::boundary("vdp_scan_stop_kind", U8, "vdp")
            .help("where this line's ramp ends: 0 full walk / 1 terminator / 2 fifth match"),
        FieldDef::boundary("vdp_scan_stop_index", U8, "vdp").help("the SAT index it stops at"),
        FieldDef::boundary("vdp_scan_step_from", U8, "vdp")
            .help("the counter value the latest step replaced"),
        FieldDef::boundary("vdp_scan_stepped_ago", U32, "vdp")
            .help("XTAL periods since that step, which the presented field window rides"),
        FieldDef::boundary("vdp_scan_field_hold", U8, "vdp")
            .help("the fifth match's hold on the presented field")
            .nullable(),
        FieldDef::boundary("vdp_scan_fifth_match", Bool, "vdp")
            .help("this scan hit a fifth match, so the hold survives the reset"),
        FieldDef::boundary("vdp_segment_bits", U8, "vdp").help("the latched fetch's pattern byte"),
        FieldDef::boundary("vdp_segment_foreground", U8, "vdp"),
        FieldDef::boundary("vdp_segment_background", U8, "vdp"),
        FieldDef::boundary("vdp_segment_start_x", U16, "vdp"),
        FieldDef::boundary("vdp_segment_end_x", U16, "vdp"),
    ]);

    for (counter, output) in [
        ("psg_tone1_counter", "psg_tone1_output"),
        ("psg_tone2_counter", "psg_tone2_output"),
        ("psg_tone3_counter", "psg_tone3_output"),
    ] {
        fields.push(FieldDef::boundary(counter, U16, "psg").help("the counter toward its borrow"));
        fields.push(FieldDef::boundary(output, Bool, "psg").help("the frequency flip-flop"));
    }
    fields.extend([
        FieldDef::boundary("psg_noise_counter", U16, "psg").help("the counter toward its borrow"),
        FieldDef::boundary("psg_noise_output", Bool, "psg")
            .help("the flip-flop clocking the shift register"),
        FieldDef::boundary("psg_noise_shift_register", U16, "psg")
            .help("the noise shift register's contents"),
        FieldDef::boundary("psg_clock_divider", U8, "psg")
            .help("the ÷16 prescaler's count toward an internal clock"),
        FieldDef::boundary("psg_ready_countdown", U8, "psg")
            .help("input clocks left of the byte load holding READY low"),
    ]);

    fields.extend([
        FieldDef::boundary("audio_sample_phase", U32, "board")
            .help("the 44.1 kHz output tap's carried phase, in T-states — an output stage, not board silicon")
            .sourced("missingno")
            .nullable(),
        FieldDef::boundary("fields_taken", U32, "board")
            .help("rasters the board has handed out, so a completed one is not handed out twice")
            .sourced("missingno"),
    ]);

    fields
}

/// The byte regions a save state carries: the work RAM, whatever RAM the
/// cartridge holds, the VDP's DRAM, and the two line buffers under the raster.
/// The field being emitted travels as the state's framebuffer, since a mid-field
/// save cannot reconstruct the rows already put down.
fn memory_spans() -> Vec<MemorySpan> {
    vec![
        MemorySpan::addressable("work_ram", RAM_BASE, RAM_SIZE)
            .help("the TMM2009's kilobyte, before the decode mirrors it"),
        // Where a board's RAM answers and how much of it there is are the
        // board's own decode, so it travels as one linear region.
        MemorySpan::off_bus("cart_ram", 0)
            .optional()
            .help("the cartridge's RAM chips, in the order the board decodes them"),
        MemorySpan::off_bus("vram", VRAM_SIZE as u32).help("the VDP's DRAM, in physical order"),
        MemorySpan::off_bus("vdp_line", VISIBLE_WIDTH as u32)
            .help("the row being composited under the raster"),
        MemorySpan::off_bus("vdp_sprite_plane", SPRITE_PLANE_WIDTH)
            .help("the line-latched sprite plane of the row being emitted"),
    ]
}

fn visible_lines() -> u32 {
    STANDARD.visible_lines() as u32
}

/// The picture the console hands out: the VDP's visible raster — the display
/// area inside its backdrop border — as TI colour indices.
fn frame() -> FrameSpec {
    FrameSpec {
        width: VISIBLE_WIDTH as u32,
        height: Some(visible_lines()),
        format: PixelFormat::Indexed8,
    }
}

static SG1000_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| {
    let mut fields = observable_fields();
    fields.extend(boundary_fields());
    SystemStateSchema {
        system: "sg1000",
        fields,
        memory: memory_spans(),
        frame: frame(),
    }
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
