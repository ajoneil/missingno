//! The trace-capture bridge: a `.morepork` execution trace authored from the
//! core's hardware-named [`SystemStateSchema`], not from a catalogue hand-mirrored
//! beside it. Each captured column is either a schema field (its type, subsystem,
//! and tier come straight from the schema) or a trace-only observation the schema
//! deliberately excludes because it is re-derivable at a boundary — the executing
//! instruction address, the pixel output, and the VRAM/APU write taps.
//!
//! The state a column carries is read through the same [`ConsoleUi::read_state`]
//! bridge the save-state framing uses, so a trace column and a save-state field
//! are the *same* hardware quantity produced the same way — one vocabulary, two
//! framings. The pixel-pipeline cells the boundary record omits (they are idle at
//! a boundary) come from the PPU's per-tick trace snapshot when the deep scope
//! opts them in.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::{HeaderFieldDef, PixFormat, TraceHeader};
use morepork::profile::FieldType as WireType;
// `Profile` is re-exported for the frontend's multi-system trace command, which
// shares one profile type across the console families. The Game Boy schema-driven
// capture reads only the trigger cadence from it — the column set comes from the
// state schema, not the profile — while the 6502 cores still consume the full
// profile.
pub use morepork::{BootRom, Profile, Trigger};
use sha2::{Digest, Sha256};

use missingno_core::state::{
    FieldType, PixelFormat, Provenance, StateValue, SystemStateSchema, Tier,
};

use crate::Console;
use crate::ppu::{PpuTraceSnapshot, TracePixel};
use crate::system::ConsoleUi;

/// Which tier of the schema a trace captures. The observable surface is the
/// cross-emulator comparison ground; the full scope adds the boundary-complete
/// deep state (a gate-level producer fills nearly all of it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TraceScope {
    /// Tier-1 observable fields plus the trace observations. The diff surface.
    #[default]
    Observable,
    /// Also the schema's Tier-2a deep state: the scalar counters/latches and the
    /// pixel-pipeline cells.
    Full,
}

/// A pixel-pipeline cell read from the PPU's per-tick trace snapshot. These are
/// the schema's nullable Tier-2a fields the boundary record does not carry.
#[derive(Clone, Copy)]
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
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "bgw_fifo_a" => Self::BgwFifoA,
            "bgw_fifo_b" => Self::BgwFifoB,
            "spr_fifo_a" => Self::SprFifoA,
            "spr_fifo_b" => Self::SprFifoB,
            "pal_pipe" => Self::PalPipe,
            "tfetch_state" => Self::TfetchState,
            "sfetch_state" => Self::SfetchState,
            "tile_temp_a" => Self::TileTempA,
            "tile_temp_b" => Self::TileTempB,
            "pix_count" => Self::PixCount,
            "sprite_count" => Self::SpriteCount,
            "scan_count" => Self::ScanCount,
            "rendering" => Self::Rendering,
            "win_mode" => Self::WinMode,
            _ => return None,
        })
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
    VramAddr,
    VramData,
    ApuWriteAddr,
    ApuWriteData,
}

struct ObservationDef {
    name: &'static str,
    ty: FieldType,
    subsystem: &'static str,
    layer: &'static str,
    nullable: bool,
    observation: Observation,
}

/// The trace observations, in capture order. `op_addr` leads so it is the
/// instruction-address column; `pix` and the write taps follow.
static OBSERVATIONS: &[ObservationDef] = &[
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
    ObservationDef {
        name: "vram_addr",
        ty: FieldType::U16,
        subsystem: "ppu",
        layer: "writes",
        nullable: true,
        observation: Observation::VramAddr,
    },
    ObservationDef {
        name: "vram_data",
        ty: FieldType::U8,
        subsystem: "ppu",
        layer: "writes",
        nullable: true,
        observation: Observation::VramData,
    },
    ObservationDef {
        name: "apu_write_addr",
        ty: FieldType::U16,
        subsystem: "apu",
        layer: "writes",
        nullable: true,
        observation: Observation::ApuWriteAddr,
    },
    ObservationDef {
        name: "apu_write_data",
        ty: FieldType::U8,
        subsystem: "apu",
        layer: "writes",
        nullable: true,
        observation: Observation::ApuWriteData,
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
    vram_write_addr: u16,
    vram_write_data: u8,
    apu_write_addr: u16,
    apu_write_data: u8,
}

fn wire_type(ty: FieldType) -> WireType {
    match ty {
        FieldType::Bool => WireType::Bool,
        FieldType::U8 => WireType::UInt8,
        FieldType::U16 => WireType::UInt16,
        // The wire format has no 32-bit column; a u32 field widens to u64.
        FieldType::U32 => WireType::UInt64,
        FieldType::Str => WireType::Str,
    }
}

fn tier_layer(tier: Tier) -> &'static str {
    match tier {
        Tier::Observable => "registers",
        Tier::Boundary => "internal",
        Tier::Tick => "timing",
    }
}

fn pix_format(format: PixelFormat) -> PixFormat {
    match format {
        PixelFormat::Shade2 => PixFormat::Shade2,
        PixelFormat::Rgb555 => PixFormat::Rgb555,
        PixelFormat::Indexed8 => PixFormat::Indexed8,
    }
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

        let (columns, field_defs) = build_columns(schema, scope);
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

        let field_names: Vec<String> = field_defs.iter().map(|d| d.name.clone()).collect();

        let header = TraceHeader {
            _header: true,
            format_version: "0.1.0".into(),
            emulator: "missingno".into(),
            emulator_version: env!("CARGO_PKG_VERSION").into(),
            rom_sha256,
            model: model_label.into(),
            boot_rom,
            profile: match scope {
                TraceScope::Observable => "observable".into(),
                TraceScope::Full => "full".into(),
            },
            fields: field_names,
            trigger: trigger.clone(),
            system: schema.system.into(),
            isa: "sm83".into(),
            pix_format: pix_format(schema.frame.format),
            field_defs,
            instruction_addr_field: Some("op_addr".into()),
            snapshot_kinds: vec!["frame".into(), "memory".into()],
            notes: String::new(),
            ..Default::default()
        };

        let writer = MoreporkWriter::create(path, &header, &[])?;

        Ok(Self {
            writer,
            columns,
            needs_pipeline,
            tcycle_count: 0,
            trigger,
            pix_buffer: String::new(),
            vram_write_addr: 0,
            vram_write_data: 0,
            apu_write_addr: 0,
            apu_write_data: 0,
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

    pub fn push_vram_write(&mut self, addr: u16, data: u8) {
        self.vram_write_addr = addr;
        self.vram_write_data = data;
    }

    pub fn push_apu_write(&mut self, addr: u16, data: u8) {
        self.apu_write_addr = addr;
        self.apu_write_data = data;
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
        let taps = ObsTaps {
            op_addr: gb.cpu().ir_address,
            vram_addr: self.vram_write_addr,
            vram_data: self.vram_write_data,
            apu_addr: self.apu_write_addr,
            apu_data: self.apu_write_data,
        };

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
                Source::Obs(observation) => emit_obs(
                    w,
                    col,
                    *observation,
                    column.nullable,
                    &taps,
                    pix_buffer,
                    &ppu_snap,
                ),
            }
        }

        self.pix_buffer.clear();
        self.vram_write_addr = 0;
        self.vram_write_data = 0;
        self.apu_write_addr = 0;
        self.apu_write_data = 0;

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

/// Build the ordered column plan and the matching header field defs for a scope.
/// Tier-1 (and, under `Full`, Tier-2a) schema fields lead; the observations
/// follow. Every column's type metadata comes from the schema or the observation
/// table — never re-authored here.
fn build_columns(
    schema: &SystemStateSchema,
    scope: TraceScope,
) -> (Vec<Column>, Vec<HeaderFieldDef>) {
    let mut columns = Vec::new();
    let mut defs = Vec::new();

    for field in &schema.fields {
        let include = match field.tier {
            Tier::Observable => true,
            Tier::Boundary => scope == TraceScope::Full,
            Tier::Tick => false,
        };
        if !include {
            continue;
        }

        let source = match PipelineCell::from_name(field.name) {
            Some(cell) => Source::Pipeline(cell),
            None => Source::Field(field.name),
        };
        columns.push(Column {
            ty: field.ty,
            nullable: field.nullable,
            source,
        });
        defs.push(HeaderFieldDef {
            name: field.name.to_string(),
            field_type: wire_type(field.ty),
            subsystem: Some(field.subsystem.to_string()),
            layer: Some(tier_layer(field.tier).to_string()),
            nullable: field.nullable,
            dictionary: false,
            source: match field.provenance {
                Provenance::Hardware => None,
                Provenance::Emulator(id) => Some(id.to_string()),
            },
        });
    }

    for obs in OBSERVATIONS {
        columns.push(Column {
            ty: obs.ty,
            nullable: obs.nullable,
            source: Source::Obs(obs.observation),
        });
        defs.push(HeaderFieldDef {
            name: obs.name.to_string(),
            field_type: wire_type(obs.ty),
            subsystem: Some(obs.subsystem.to_string()),
            layer: Some(obs.layer.to_string()),
            nullable: obs.nullable,
            dictionary: false,
            source: Some("missingno".to_string()),
        });
    }

    (columns, defs)
}

fn emit_value(
    w: &mut MoreporkWriter,
    col: usize,
    ty: FieldType,
    nullable: bool,
    value: Option<&StateValue>,
) {
    match value {
        Some(StateValue::Int(v)) => match ty {
            FieldType::U8 => w.set_u8(col, *v as u8),
            FieldType::U16 => w.set_u16(col, *v as u16),
            FieldType::U32 => w.set_u64(col, *v as u64),
            _ => w.set_u8(col, *v as u8),
        },
        Some(StateValue::Bool(b)) => w.set_bool(col, *b),
        Some(StateValue::Text(t)) => w.set_str(col, t),
        Some(StateValue::Null) | None => {
            if nullable {
                w.set_null(col);
            } else {
                // A non-nullable schema field the record did not carry: emit a
                // type-appropriate zero so columns stay aligned.
                match ty {
                    FieldType::Bool => w.set_bool(col, false),
                    FieldType::U16 => w.set_u16(col, 0),
                    FieldType::U32 => w.set_u64(col, 0),
                    FieldType::Str => w.set_str(col, ""),
                    FieldType::U8 => w.set_u8(col, 0),
                }
            }
        }
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

/// The per-step write taps and instruction address, snapshotted before the
/// column loop so the emitter borrows no `self` field but the writer.
struct ObsTaps {
    op_addr: u16,
    vram_addr: u16,
    vram_data: u8,
    apu_addr: u16,
    apu_data: u8,
}

fn emit_obs(
    w: &mut MoreporkWriter,
    col: usize,
    observation: Observation,
    nullable: bool,
    taps: &ObsTaps,
    pix_buffer: &str,
    ppu_snap: &Option<PpuTraceSnapshot>,
) {
    let ObsTaps {
        op_addr,
        vram_addr,
        vram_data,
        apu_addr,
        apu_data,
    } = *taps;
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
        Observation::VramAddr => {
            if vram_addr != 0 {
                w.set_u16(col, vram_addr);
            } else if nullable {
                w.set_null(col);
            } else {
                w.set_u16(col, 0);
            }
        }
        Observation::VramData => {
            if vram_addr != 0 {
                w.set_u8(col, vram_data);
            } else if nullable {
                w.set_null(col);
            } else {
                w.set_u8(col, 0);
            }
        }
        Observation::ApuWriteAddr => {
            if apu_addr != 0 {
                w.set_u16(col, apu_addr);
            } else if nullable {
                w.set_null(col);
            } else {
                w.set_u16(col, 0);
            }
        }
        Observation::ApuWriteData => {
            if apu_addr != 0 {
                w.set_u8(col, apu_data);
            } else if nullable {
                w.set_null(col);
            } else {
                w.set_u8(col, 0);
            }
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
