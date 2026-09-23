//! The trace-capture bridge: a `.morepork` execution trace authored from the
//! console's hardware-named [`SystemStateSchema`](missingno_core::state::SystemStateSchema),
//! one entry per instruction boundary. Each column is either a schema field or
//! a trace-only observation the schema excludes because it is not machine
//! state — the T-states since the last entry, the raster position, the corpus
//! RESULT block, and the last work-RAM write.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::PixFormat;
use morepork::snapshot::IndexedFrame;

use missingno_core::machine::rom_fingerprint;
use missingno_core::state::FieldType;
use missingno_ti_vdp::{ACTIVE_LINES, ACTIVE_WIDTH, Frame, LEFT_BORDER, PALETTE, Standard};
use missingno_trace::{
    BootRom, Column, ObservationDef, Source, TraceIdentity, build_columns, create_writer,
    emit_value,
};
pub use missingno_trace::{TraceScope, Trigger};

use crate::console::ColecoVision;
use crate::debug::pixel_aspect;
use crate::snapshot::read_state;
use crate::state_schema::colecovision_state_schema;

/// A trace-only observation: a per-step surface the state schema excludes
/// because it is not machine state. Bridge-owned, marked `missingno`-sourced.
#[derive(Clone, Copy)]
enum Observation {
    /// T-states consumed since the previous entry.
    Cycles,
    /// The VDP raster's counter line and dot at the entry.
    Line,
    Dot,
    /// The corpus RESULT block.
    Result,
    Code,
    Observed,
    Expected,
    /// The last work-RAM write since the previous entry, or null.
    RamWriteAddr,
    RamWriteData,
}

/// The trace observations, in capture order, named as the corpus reads them.
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
        subsystem: "vdp",
        layer: "timing",
        nullable: false,
        observation: Observation::Line,
    },
    ObservationDef {
        name: "dot",
        ty: FieldType::U16,
        subsystem: "vdp",
        layer: "timing",
        nullable: false,
        observation: Observation::Dot,
    },
    ObservationDef {
        name: "result",
        ty: FieldType::U8,
        subsystem: "board",
        layer: "registers",
        nullable: false,
        observation: Observation::Result,
    },
    ObservationDef {
        name: "code",
        ty: FieldType::U8,
        subsystem: "board",
        layer: "registers",
        nullable: false,
        observation: Observation::Code,
    },
    ObservationDef {
        name: "observed",
        ty: FieldType::U8,
        subsystem: "board",
        layer: "registers",
        nullable: false,
        observation: Observation::Observed,
    },
    ObservationDef {
        name: "expected",
        ty: FieldType::U8,
        subsystem: "board",
        layer: "registers",
        nullable: false,
        observation: Observation::Expected,
    },
    ObservationDef {
        name: "ram_write_addr",
        ty: FieldType::U16,
        subsystem: "board",
        layer: "registers",
        nullable: true,
        observation: Observation::RamWriteAddr,
    },
    ObservationDef {
        name: "ram_write_data",
        ty: FieldType::U8,
        subsystem: "board",
        layer: "registers",
        nullable: true,
        observation: Observation::RamWriteData,
    },
];

/// Where the corpus RESULT block lives on this board.
const RESULT_BASE: u16 = 0x7000;

/// The hex spelling of a digest, as a trace's media binding carries it.
pub(crate) fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn part_name(standard: Standard) -> &'static str {
    match standard {
        Standard::Ntsc => "TMS9918A",
        Standard::Pal => "TMS9929A",
    }
}

/// Captures `.morepork` execution traces from a ColecoVision, keyed on the
/// console's hardware state schema.
pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column<Observation>>,
    standard: Standard,
}

impl Tracer {
    pub fn create(
        path: impl AsRef<Path>,
        rom: &[u8],
        standard: Standard,
        trigger: Trigger,
        scope: TraceScope,
    ) -> Result<Tracer, morepork::Error> {
        Tracer::create_hashed(
            path,
            hex_digest(&rom_fingerprint(rom)),
            standard,
            trigger,
            scope,
        )
    }

    /// [`Tracer::create`] for a caller that holds the ROM's hash but not its
    /// bytes.
    pub fn create_hashed(
        path: impl AsRef<Path>,
        rom_sha256: String,
        standard: Standard,
        trigger: Trigger,
        scope: TraceScope,
    ) -> Result<Tracer, morepork::Error> {
        let schema = colecovision_state_schema();
        let (columns, field_defs) = build_columns(schema, scope, OBSERVATIONS);

        let writer = create_writer(
            path,
            TraceIdentity {
                rom_sha256,
                // morepork's catalogue names this system `coleco`.
                system: "coleco",
                isa: "z80",
                model: part_name(standard),
                scope,
                trigger,
                pix_format: PixFormat::Indexed8,
                boot_rom: BootRom::Builtin,
                instruction_addr_field: "pc",
                snapshot_kinds: vec!["frame".into()],
            },
            field_defs,
        )?;
        Ok(Tracer {
            writer,
            columns,
            standard,
        })
    }

    /// Write one entry from the console's current state, taking the work-RAM
    /// write it has held since the previous entry.
    pub fn capture(&mut self, cv: &mut ColecoVision) -> Result<(), morepork::Error> {
        let ram_write = cv.take_ram_write();
        let record = read_state(cv);
        let cycles = cv.cpu.bus_trace().len() as u16;
        let (line, dot) = (cv.vdp().line(), cv.vdp().dot());
        let ram = cv.ram();
        let result = |offset: usize| ram[(RESULT_BASE as usize + offset) & (ram.len() - 1)];
        let w = &mut self.writer;
        for (col, column) in self.columns.iter().enumerate() {
            match &column.source {
                Source::Field(name) => emit_value(
                    w,
                    col,
                    column.ty,
                    column.nullable,
                    record.as_ref().and_then(|record| record.get(name)),
                ),
                Source::Observation(Observation::Cycles) => w.set_u16(col, cycles),
                Source::Observation(Observation::Line) => w.set_u16(col, line),
                Source::Observation(Observation::Dot) => w.set_u16(col, dot),
                Source::Observation(Observation::Result) => w.set_u8(col, result(0)),
                Source::Observation(Observation::Code) => w.set_u8(col, result(1)),
                Source::Observation(Observation::Observed) => w.set_u8(col, result(2)),
                Source::Observation(Observation::Expected) => w.set_u8(col, result(3)),
                Source::Observation(Observation::RamWriteAddr) => match ram_write {
                    Some((address, _)) => w.set_u16(col, address),
                    None => w.set_null(col),
                },
                Source::Observation(Observation::RamWriteData) => match ram_write {
                    Some((_, data)) => w.set_u8(col, data),
                    None => w.set_null(col),
                },
            }
        }
        self.writer.finish_entry()
    }

    /// Record a frame boundary, with the completed frame's display area as an
    /// indexed snapshot under the chip's canonical palette.
    pub fn mark_frame(&mut self, frame: Option<&Frame>) -> Result<(), morepork::Error> {
        let payload = frame.map(|frame| display_area(frame, self.standard).to_bytes());
        self.writer.mark_frame(payload.as_deref())
    }

    pub fn finish(self) -> Result<(), morepork::Error> {
        self.writer.finish()
    }
}

/// The 256×192 display area cropped out of the visible raster.
fn display_area(frame: &Frame, standard: Standard) -> IndexedFrame {
    let width = frame.width as usize;
    let top = standard.top_border() as usize;
    IndexedFrame {
        width: ACTIVE_WIDTH,
        height: ACTIVE_LINES,
        pixel_aspect: pixel_aspect(match standard {
            Standard::Ntsc => missingno_core::TvStandard::Ntsc,
            Standard::Pal => missingno_core::TvStandard::Pal,
        }),
        palette: PALETTE.to_vec(),
        pixels: (0..ACTIVE_LINES as usize)
            .flat_map(|y| {
                let start = (top + y) * width + LEFT_BORDER as usize;
                frame.pixels[start..start + ACTIVE_WIDTH as usize]
                    .iter()
                    .copied()
            })
            .collect(),
    }
}
