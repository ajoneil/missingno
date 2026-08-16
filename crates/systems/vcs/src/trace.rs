//! The trace-capture bridge: a `.morepork` execution trace authored from the
//! console's hardware-named [`SystemStateSchema`], not from a catalogue mirrored
//! beside it. Each captured column is either a schema field (its type, subsystem,
//! and tier come straight from the schema) or a trace-only observation the schema
//! deliberately excludes because it is not machine state — the CPU-cycle delta
//! since the last entry and the emergent scanline index.
//!
//! Column values are read through the same [`read_state`] bridge the save-state
//! framing uses, so a trace column and a save-state field are the *same*
//! hardware quantity produced the same way — one vocabulary, two framings.
//! Frames are emergent from the software's sync pattern, so every frame snapshot
//! carries its own height as an [`IndexedFrame`].

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::PixFormat;
use sha2::{Digest, Sha256};

use missingno_core::state::FieldType;
use missingno_core::trace::{
    BootRom, Column, ObservationDef, Source, TraceIdentity, build_columns, create_writer,
    emit_value,
};
pub use missingno_core::trace::{TraceScope, Trigger};

use crate::TvStandard;
use crate::console::{Frame, Vcs};
use crate::snapshot::read_state;
use crate::state_schema::vcs_state_schema;
use crate::tia::{VISIBLE_CLOCKS, palette, palette_index};
use crate::tv_standard::pixel_aspect;

use morepork::snapshot::IndexedFrame;

/// A trace-only observation: a per-step surface the state schema excludes
/// because it is not machine state. Bridge-owned, marked `missingno`-sourced.
#[derive(Clone, Copy)]
enum Observation {
    /// CPU cycles consumed since the previous entry — a WSYNC stall parks the
    /// CPU, so one store can span most of a scanline.
    Cycles,
    /// The emergent scanline index within the field (frame-assembly count).
    Line,
}

/// The trace observations, in capture order.
static OBSERVATIONS: &[ObservationDef<Observation>] = &[
    ObservationDef {
        name: "cycles",
        ty: FieldType::U16,
        subsystem: "cpu",
        layer: "timing",
        nullable: false,
        observation: Observation::Cycles,
    },
    ObservationDef {
        name: "line",
        ty: FieldType::U16,
        subsystem: "tia",
        layer: "timing",
        nullable: false,
        observation: Observation::Line,
    },
];

/// Run to the next instruction boundary, returning the CPU cycles consumed
/// (WSYNC parks the CPU, so one store can span most of a scanline). The tracing
/// counterpart of [`Vcs::step_instruction`].
pub fn step_instruction_counted(vcs: &mut Vcs) -> u16 {
    let mut cycles = 0u16;
    while vcs.at_instruction_boundary() && !vcs.cpu.jammed() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    while !vcs.at_instruction_boundary() && !vcs.cpu.jammed() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    cycles
}

/// Captures `.morepork` execution traces from a VCS console, keyed on the
/// console's hardware state schema.
pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column<Observation>>,
    region: TvStandard,
}

impl Tracer {
    /// Create a tracer whose columns are authored from the console's state
    /// schema. `scope` selects the tier depth; `trigger` records the capture
    /// cadence in the header for downstream alignment.
    pub fn create(
        path: impl AsRef<Path>,
        rom: &[u8],
        region: TvStandard,
        trigger: Trigger,
        scope: TraceScope,
    ) -> Result<Tracer, morepork::Error> {
        let rom_sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(rom);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        Tracer::create_hashed(path, rom_sha256, region, trigger, scope)
    }

    /// [`Tracer::create`] for a caller that holds the ROM's hash but not its
    /// bytes (the seam debugger fingerprints the ROM at load and drops it).
    pub fn create_hashed(
        path: impl AsRef<Path>,
        rom_sha256: String,
        region: TvStandard,
        trigger: Trigger,
        scope: TraceScope,
    ) -> Result<Tracer, morepork::Error> {
        let schema = vcs_state_schema();
        let (columns, field_defs) = build_columns(schema, scope, OBSERVATIONS);

        let writer = create_writer(
            path,
            TraceIdentity {
                rom_sha256,
                system: schema.system,
                isa: "6502",
                model: region.name(),
                scope,
                trigger,
                pix_format: PixFormat::Indexed8,
                boot_rom: BootRom::default(),
                instruction_addr_field: "pc",
                snapshot_kinds: vec!["frame".into()],
            },
            field_defs,
        )?;
        Ok(Tracer {
            writer,
            columns,
            region,
        })
    }

    /// Write one entry from the console's current state. `cycles` is the
    /// CPU-cycle delta since the previous entry.
    pub fn capture(&mut self, vcs: &Vcs, cycles: u16) -> Result<(), morepork::Error> {
        let record = read_state(vcs);
        let line = vcs.scanline() as u16;
        let w = &mut self.writer;
        for (col, column) in self.columns.iter().enumerate() {
            match &column.source {
                Source::Field(name) => {
                    emit_value(w, col, column.ty, column.nullable, record.get(name));
                }
                Source::Observation(Observation::Cycles) => w.set_u16(col, cycles),
                Source::Observation(Observation::Line) => w.set_u16(col, line),
            }
        }
        self.writer.finish_entry()
    }

    /// Record a frame boundary, with the completed frame as an indexed snapshot.
    /// Height is whatever the kernel produced; the palette rides along so the
    /// payload is self-contained.
    pub fn mark_frame(&mut self, frame: Option<&Frame>) -> Result<(), morepork::Error> {
        let payload = frame.map(|frame| {
            IndexedFrame {
                width: VISIBLE_CLOCKS as u16,
                height: frame.lines.len() as u16,
                pixel_aspect: pixel_aspect(self.region),
                palette: palette(self.region)
                    .iter()
                    .map(|&(r, g, b)| [r, g, b])
                    .collect(),
                pixels: frame
                    .lines
                    .iter()
                    .flat_map(|line| line.iter().map(|&pixel| palette_index(pixel) as u8))
                    .collect(),
            }
            .to_bytes()
        });
        self.writer.mark_frame(payload.as_deref())
    }

    pub fn finish(self) -> Result<(), morepork::Error> {
        self.writer.finish()
    }
}
