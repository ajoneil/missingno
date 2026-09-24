//! The trace-capture bridge: a `.morepork` execution trace authored from the
//! console's hardware-named [`SystemStateSchema`](missingno_core::state::SystemStateSchema),
//! one entry per instruction boundary. Each column is either a schema field or
//! a trace-only observation the schema excludes because it is not machine
//! state — the T-states since the last entry, the corpus RESULT block, and the
//! last work-RAM write.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::PixFormat;
use morepork::snapshot::IndexedFrame;

use missingno_core::machine::rom_fingerprint;
use missingno_ti_vdp::{ACTIVE_LINES, ACTIVE_WIDTH, Frame, LEFT_BORDER, PALETTE, Standard};
use missingno_trace::{
    BootRom, Column, Source, TRACE_OBSERVATIONS, TraceIdentity, TraceObservation, build_columns,
    create_writer, emit_value,
};
pub use missingno_trace::{TraceScope, Trigger};

use crate::console::Sg1000;
use crate::debug::pixel_aspect;
use crate::snapshot::read_state;
use crate::state_schema::sg1000_state_schema;

/// Where the corpus RESULT block lives on this board.
const RESULT_BASE: u16 = 0xC000;

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

/// Captures `.morepork` execution traces from an SG-1000, keyed on the
/// console's hardware state schema.
pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column<TraceObservation>>,
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
        let schema = sg1000_state_schema();
        let (columns, field_defs) = build_columns(schema, scope, TRACE_OBSERVATIONS);

        let writer = create_writer(
            path,
            schema,
            TraceIdentity {
                rom_sha256,
                model: part_name(standard),
                scope,
                trigger,
                pix_format: PixFormat::Indexed8,
                boot_rom: BootRom::Skip,
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
    pub fn capture(&mut self, sg: &mut Sg1000) -> Result<(), morepork::Error> {
        let ram_write = sg.take_ram_write();
        let record = read_state(sg);
        let cycles = sg.cpu.bus_trace().len() as u16;
        let ram = sg.work_ram();
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
                Source::Observation(TraceObservation::Cycles) => w.set_u16(col, cycles),
                Source::Observation(TraceObservation::Result) => w.set_u8(col, result(0)),
                Source::Observation(TraceObservation::Code) => w.set_u8(col, result(1)),
                Source::Observation(TraceObservation::Observed) => w.set_u8(col, result(2)),
                Source::Observation(TraceObservation::Expected) => w.set_u8(col, result(3)),
                Source::Observation(TraceObservation::RamWriteAddr) => match ram_write {
                    Some((address, _)) => w.set_u16(col, address),
                    None => w.set_null(col),
                },
                Source::Observation(TraceObservation::RamWriteData) => match ram_write {
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
