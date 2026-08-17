//! The trace-capture bridge: a `.morepork` execution trace authored from the
//! core's hardware-named [`SystemStateSchema`], not from a catalogue hand-mirrored
//! beside it. Each captured column is either a schema field (its type, subsystem,
//! and tier come straight from the schema) or a trace-only observation the schema
//! deliberately excludes because it is re-derivable at a boundary — the executing
//! instruction address and the pixel output.
//!
//! The state a column carries is read through the same [`ConsoleUi::read_state`]
//! bridge the save-state framing uses, so a trace column and a save-state field
//! are the *same* hardware quantity produced the same way — one vocabulary, two
//! framings. The pixel-pipeline cells the boundary record omits (they are idle at
//! a boundary) come from the PPU's per-tick trace snapshot when the deep scope
//! opts them in.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use sha2::{Digest, Sha256};

use missingno_core::state::FieldType;
pub use missingno_core::trace::{BootRom, Profile, TraceScope, Trigger};
use missingno_core::trace::{
    ObservationDef, Source as PlannedSource, TraceIdentity, build_columns, create_writer,
    emit_value, pix_format,
};

use crate::Console;
use crate::ppu::{PpuTraceSnapshot, TracePixel};
use crate::system::ConsoleUi;

/// A pixel-pipeline cell read from the PPU's per-tick trace snapshot. These are
/// the schema's nullable Tier-2a `ppu` fields the boundary record does not carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PipelineCell {
    BgwFifoA,
    BgwFifoB,
    SprFifoA,
    SprFifoB,
    PalPipe,
    TfetchState,
    SfetchState,
    TileTempA,
    TileTempB,
    PixCount,
    SpriteCount,
    ScanCount,
    Rendering,
    WinMode,
}

impl PipelineCell {
    /// Each cell paired with its schema field name — the single source of truth
    /// `from_name` reads and the schema-tie test checks against the schema, so
    /// renaming a field on either side breaks the build, never the trace data.
    const BY_NAME: [(&'static str, Self); 14] = [
        ("bgw_fifo_a", Self::BgwFifoA),
        ("bgw_fifo_b", Self::BgwFifoB),
        ("spr_fifo_a", Self::SprFifoA),
        ("spr_fifo_b", Self::SprFifoB),
        ("pal_pipe", Self::PalPipe),
        ("tfetch_state", Self::TfetchState),
        ("sfetch_state", Self::SfetchState),
        ("tile_temp_a", Self::TileTempA),
        ("tile_temp_b", Self::TileTempB),
        ("pix_count", Self::PixCount),
        ("sprite_count", Self::SpriteCount),
        ("scan_count", Self::ScanCount),
        ("rendering", Self::Rendering),
        ("win_mode", Self::WinMode),
    ];

    fn from_name(name: &str) -> Option<Self> {
        Self::BY_NAME
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, cell)| *cell)
    }
}

/// A trace-only observation: a per-step surface the state schema excludes because
/// it is re-derivable at a boundary rather than being machine state. Authored on
/// the producer and marked `missingno`-sourced in the header.
#[derive(Clone, Copy)]
enum Observation {
    /// The executing instruction's address — the stable per-instruction key diff
    /// collapses and aligns on (`pc` moves within a multi-cycle instruction).
    OpAddr,
    /// Pixels pushed since the last capture, encoded per the header's pix_format.
    Pix,
    /// Pixels pushed on the current line (the PPU's pipeline pixel count).
    PixX,
}

/// The trace observations, in capture order. `op_addr` leads so it is the
/// instruction-address column; the pixel columns follow.
static OBSERVATIONS: &[ObservationDef<Observation>] = &[
    ObservationDef {
        name: "op_addr",
        ty: FieldType::U16,
        subsystem: "cpu",
        layer: "timing",
        nullable: false,
        observation: Observation::OpAddr,
    },
    ObservationDef {
        name: "pix",
        ty: FieldType::Str,
        subsystem: "ppu",
        layer: "output",
        nullable: true,
        observation: Observation::Pix,
    },
    ObservationDef {
        name: "pix_x",
        ty: FieldType::U8,
        subsystem: "ppu",
        layer: "output",
        nullable: false,
        observation: Observation::PixX,
    },
];

/// How a column's value is produced at capture time.
enum Source {
    /// A schema field read from the per-capture boundary record by name.
    Field(&'static str),
    /// A pixel-pipeline cell from the PPU trace snapshot.
    Pipeline(PipelineCell),
    /// A trace observation.
    Obs(Observation),
}

/// One trace column: its schema/observation type (fixing the emitted width) and
/// how to produce its value.
struct Column {
    ty: FieldType,
    nullable: bool,
    source: Source,
}

/// Captures `.morepork` execution traces from a Game Boy–family console, keyed on
/// the console's hardware state schema.
pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column>,
    needs_pipeline: bool,
    tcycle_count: u64,
    trigger: Trigger,
    pix_buffer: String,
}

impl Tracer {
    /// Create a tracer whose columns are authored from the console model's state
    /// schema. `scope` selects the tier depth; `trigger` records the capture
    /// cadence in the header for downstream alignment.
    pub fn create<M: ConsoleUi>(
        path: impl AsRef<Path>,
        gb: &Console<M>,
        trigger: Trigger,
        scope: TraceScope,
        boot_rom: BootRom,
        model_label: &str,
    ) -> Result<Self, morepork::Error> {
        let schema = M::state_schema().ok_or_else(|| {
            morepork::Error::Profile("console model authors no state schema".into())
        })?;

        let (planned, field_defs) = build_columns(schema, scope, OBSERVATIONS);
        let columns: Vec<Column> = planned
            .into_iter()
            .map(|column| Column {
                ty: column.ty,
                nullable: column.nullable,
                source: match column.source {
                    // A pipeline cell is a schema field the boundary record
                    // cannot answer; the PPU's per-tick snapshot does.
                    PlannedSource::Field(name) => match PipelineCell::from_name(name) {
                        Some(cell) => Source::Pipeline(cell),
                        None => Source::Field(name),
                    },
                    PlannedSource::Observation(observation) => Source::Obs(observation),
                },
            })
            .collect();
        let needs_pipeline = columns.iter().any(|c| {
            matches!(c.source, Source::Pipeline(_))
                || matches!(c.source, Source::Obs(Observation::PixX))
        });

        let rom_sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(gb.cartridge().rom());
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };

        let writer = create_writer(
            path,
            TraceIdentity {
                rom_sha256,
                system: schema.system,
                isa: "sm83",
                model: model_label,
                scope,
                trigger: trigger.clone(),
                pix_format: pix_format(schema.frame.format),
                boot_rom,
                instruction_addr_field: "op_addr",
                snapshot_kinds: vec!["frame".into(), "memory".into()],
            },
            field_defs,
        )?;

        Ok(Self {
            writer,
            columns,
            needs_pipeline,
            tcycle_count: 0,
            trigger,
            pix_buffer: String::new(),
        })
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger.clone()
    }

    pub fn push_pixel(&mut self, shade: u8) {
        self.pix_buffer.push((b'0' + (shade & 3)) as char);
    }

    /// Push a CGB pixel as 4 hex chars (15-bit RGB555). Use this when the trace's
    /// `pix_format` is `Rgb555` (CGB/AGB models); `push_pixel` otherwise.
    pub fn push_pixel_rgb555(&mut self, value: u16) {
        use std::fmt::Write;
        let _ = write!(self.pix_buffer, "{:04X}", value & 0x7FFF);
    }

    /// Write a typed snapshot record into the trace stream. `tag` is a
    /// format-level tag (`TAG_FRAME`, `TAG_MEMORY`).
    pub fn write_snapshot(&mut self, tag: u8, payload: &[u8]) -> Result<(), morepork::Error> {
        self.writer.write_snapshot(tag, payload)
    }

    /// Capture one row: the schema fields from the boundary record, the pipeline
    /// cells from the PPU snapshot, and the observations from the taps.
    pub fn capture<M: ConsoleUi>(&mut self, gb: &Console<M>) -> Result<(), morepork::Error> {
        let record = M::read_state(gb);
        let ppu_snap = if self.needs_pipeline {
            gb.ppu().trace_snapshot()
        } else {
            None
        };
        let op_addr = gb.cpu().ir_address;

        let pix_buffer = &self.pix_buffer;
        let columns = &self.columns;
        let w = &mut self.writer;
        for (col, column) in columns.iter().enumerate() {
            match &column.source {
                Source::Field(name) => {
                    let value = record.as_ref().and_then(|r| r.get(name));
                    emit_value(w, col, column.ty, column.nullable, value);
                }
                Source::Pipeline(cell) => emit_pipeline(w, col, *cell, &ppu_snap),
                Source::Obs(observation) => {
                    emit_obs(w, col, *observation, op_addr, pix_buffer, &ppu_snap)
                }
            }
        }

        self.pix_buffer.clear();

        self.writer.finish_entry()
    }

    pub fn mark_frame(&mut self) -> Result<(), morepork::Error> {
        self.writer.mark_frame(None)
    }

    pub fn advance_dot(&mut self) {
        self.tcycle_count += 1;
    }

    pub fn advance(&mut self, tcycles: u32) {
        self.tcycle_count += tcycles as u64;
    }

    pub fn tcycle_count(&self) -> u64 {
        self.tcycle_count
    }

    pub fn finish(self) -> Result<(), morepork::Error> {
        self.writer.finish()
    }
}

fn emit_pipeline(
    w: &mut MoreporkWriter,
    col: usize,
    cell: PipelineCell,
    snap: &Option<PpuTraceSnapshot>,
) {
    let Some(snap) = snap else {
        w.set_null(col);
        return;
    };
    match cell {
        PipelineCell::BgwFifoA => w.set_u8(col, snap.bgw_fifo_a),
        PipelineCell::BgwFifoB => w.set_u8(col, snap.bgw_fifo_b),
        PipelineCell::SprFifoA => w.set_u8(col, snap.spr_fifo_a),
        PipelineCell::SprFifoB => w.set_u8(col, snap.spr_fifo_b),
        PipelineCell::PalPipe => w.set_u8(col, snap.pal_pipe),
        PipelineCell::TfetchState => w.set_u8(col, snap.tfetch_state),
        PipelineCell::SfetchState => w.set_u8(col, snap.sfetch_state),
        PipelineCell::TileTempA => w.set_u8(col, snap.tile_temp_a),
        PipelineCell::TileTempB => w.set_u8(col, snap.tile_temp_b),
        PipelineCell::PixCount => w.set_u8(col, snap.pix_count),
        PipelineCell::SpriteCount => w.set_u8(col, snap.sprite_count),
        PipelineCell::ScanCount => w.set_u8(col, snap.scan_count),
        PipelineCell::Rendering => w.set_bool(col, snap.rendering),
        PipelineCell::WinMode => w.set_bool(col, snap.win_mode),
    }
}

fn emit_obs(
    w: &mut MoreporkWriter,
    col: usize,
    observation: Observation,
    op_addr: u16,
    pix_buffer: &str,
    ppu_snap: &Option<PpuTraceSnapshot>,
) {
    match observation {
        Observation::OpAddr => w.set_u16(col, op_addr),
        Observation::Pix => {
            if pix_buffer.is_empty() {
                w.set_null(col);
            } else {
                w.set_str(col, pix_buffer);
            }
        }
        Observation::PixX => {
            let count = ppu_snap.as_ref().map(|s| s.pix_count).unwrap_or(0);
            w.set_u8(col, count);
        }
    }
}

/// Step one instruction dot-by-dot, capturing trace state at every CPU T-cycle
/// and resolving STOP / VRAM-DMA holds at the boundary like `step`. The shared
/// driver behind every tcycle-triggered capture.
pub fn step_instruction_tcycle<M: ConsoleUi>(
    gb: &mut Console<M>,
    tracer: &mut Tracer,
) -> crate::execute::StepResult {
    let mut new_screen = false;
    let mut tcycles = 0u32;

    gb.cpu_mut().bus.data_latch = 0;
    gb.cpu_mut().take_instruction_boundary();

    // Speed is fixed across one instruction; a mid-instruction switch settles at
    // the boundary in `resolve_stop`. Single speed captures once per T-cycle
    // (after the fall, combining both edges' frame flag); double speed captures
    // after every master edge — the CPU runs at 2× the dot clock — and may retire
    // mid-pair, deferring the fall to the next call.
    let double_speed = gb.cpu_steps_per_dot() == 2;

    loop {
        let mut first_new_screen = false;
        let mut is_first = true;
        gb.execute_tcycle_observed(|gb, result| {
            new_screen |= result.new_screen;
            if let Some(pixel) = result.pixel {
                match pixel.pixel {
                    TracePixel::Shade(shade) => tracer.push_pixel(shade),
                    TracePixel::Rgb555(color) => tracer.push_pixel_rgb555(color),
                }
            }
            if double_speed {
                if result.new_screen {
                    tracer.mark_frame().unwrap();
                }
                tracer.capture(gb).unwrap();
                tracer.advance_dot();
                tcycles += 1;
            } else if is_first {
                first_new_screen = result.new_screen;
                is_first = false;
            } else {
                if first_new_screen || result.new_screen {
                    tracer.mark_frame().unwrap();
                }
                tracer.capture(gb).unwrap();
                tracer.advance_dot();
                tcycles += 1;
            }
        });

        if gb.cpu().at_instruction_boundary() {
            break;
        }
    }

    // Mirror `step`: resolve a settled STOP (CGB speed-switch blackout) and
    // engage/release a VRAM-DMA CPU hold, so traced runs progress past STOP and
    // run their DMAs like untraced ones.
    gb.resolve_stop(tcycles);
    gb.manage_dma_hold();

    crate::execute::StepResult {
        new_screen,
        tcycles,
        sram_dirty: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ConsoleUi;
    use missingno_core::state::Tier;

    /// The pipeline-cell table and the schema must stay in lockstep: every cell
    /// names a nullable Tier-2a `ppu` field, and every such field resolves to a
    /// cell. A rename on either side then fails here rather than silently
    /// dropping a column to nulls.
    #[test]
    fn pipeline_cells_match_the_schema_ppu_boundary_fields() {
        let schema = <crate::Dmg as ConsoleUi>::state_schema()
            .expect("the DMG model authors a state schema");

        // Forward: every cell names a nullable Tier-2a `ppu` field.
        for (name, _) in PipelineCell::BY_NAME {
            let field = schema
                .fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("pipeline cell `{name}` has no schema field"));
            assert!(
                matches!(field.tier, Tier::Boundary),
                "`{name}` must be a Tier-2a (boundary) field"
            );
            assert!(field.nullable, "`{name}` must be nullable");
            assert_eq!(field.subsystem, "ppu", "`{name}` must be a `ppu` field");
        }

        // Reverse: every nullable Tier-2a `ppu` field is a known cell, so adding
        // one to the schema without wiring a cell breaks here.
        for field in schema
            .fields
            .iter()
            .filter(|f| f.subsystem == "ppu" && matches!(f.tier, Tier::Boundary) && f.nullable)
        {
            assert!(
                PipelineCell::from_name(field.name).is_some(),
                "nullable `ppu` boundary field `{}` has no PipelineCell",
                field.name
            );
        }
    }
}
