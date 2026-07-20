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
use morepork::header::{HeaderFieldDef, PixFormat, TraceHeader};
use morepork::profile::FieldType as WireType;
pub use morepork::{BootRom, Trigger};
use sha2::{Digest, Sha256};

use missingno_core::state::{FieldType, Provenance, StateValue, SystemStateSchema, Tier};

use crate::TvStandard;
use crate::console::{Frame, Vcs};
use crate::snapshot::read_state;
use crate::state_schema::vcs_state_schema;
use crate::tia::{VISIBLE_CLOCKS, palette, palette_index};
use crate::tv_standard::PIXEL_ASPECT;

use morepork::snapshot::IndexedFrame;

/// Which tier of the schema a trace captures. The observable surface is the
/// cross-emulator comparison ground; the full scope adds the boundary-complete
/// deep state (object counters, ring phases, motion engine, audio counters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TraceScope {
    /// Tier-1 observable fields plus the trace observations. The diff surface.
    #[default]
    Observable,
    /// Also the schema's Tier-2a deep die state.
    Full,
}

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

struct ObservationDef {
    name: &'static str,
    ty: FieldType,
    subsystem: &'static str,
    observation: Observation,
}

/// The trace observations, in capture order.
static OBSERVATIONS: &[ObservationDef] = &[
    ObservationDef {
        name: "cycles",
        ty: FieldType::U16,
        subsystem: "cpu",
        observation: Observation::Cycles,
    },
    ObservationDef {
        name: "line",
        ty: FieldType::U16,
        subsystem: "tia",
        observation: Observation::Line,
    },
];

/// How a column's value is produced at capture time.
enum Source {
    /// A schema field read from the per-capture record by name.
    Field(&'static str),
    /// A trace observation.
    Obs(Observation),
}

struct Column {
    ty: FieldType,
    nullable: bool,
    source: Source,
}

/// Run to the next instruction boundary, returning the CPU cycles consumed
/// (WSYNC parks the CPU, so one store can span most of a scanline). The tracing
/// counterpart of [`Vcs::step_instruction`].
pub fn step_instruction_counted(vcs: &mut Vcs) -> u16 {
    let mut cycles = 0u16;
    while vcs.at_instruction_boundary() && !vcs.cpu.halted() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    while !vcs.at_instruction_boundary() && !vcs.cpu.halted() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    cycles
}

/// Captures `.morepork` execution traces from a VCS console, keyed on the
/// console's hardware state schema.
pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column>,
    region: TvStandard,
}

fn wire_type(ty: FieldType) -> WireType {
    match ty {
        FieldType::Bool => WireType::Bool,
        FieldType::U8 => WireType::UInt8,
        FieldType::U16 => WireType::UInt16,
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
        let schema = vcs_state_schema();
        let (columns, field_defs) = build_columns(schema, scope);

        let rom_sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(rom);
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
            system: schema.system.into(),
            isa: "6502".into(),
            model: region.name().into(),
            profile: match scope {
                TraceScope::Observable => "observable".into(),
                TraceScope::Full => "full".into(),
            },
            fields: field_names,
            trigger,
            pix_format: PixFormat::Indexed8,
            field_defs,
            instruction_addr_field: Some("pc".into()),
            snapshot_kinds: vec!["frame".into()],
            notes: String::new(),
            ..Default::default()
        };

        let writer = MoreporkWriter::create(path, &header, &[])?;
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
                Source::Obs(Observation::Cycles) => w.set_u16(col, cycles),
                Source::Obs(Observation::Line) => w.set_u16(col, line),
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
                pixel_aspect: PIXEL_ASPECT,
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

/// Build the ordered column plan and matching header field defs for a scope.
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
        columns.push(Column {
            ty: field.ty,
            nullable: field.nullable,
            source: Source::Field(field.name),
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
            nullable: false,
            source: Source::Obs(obs.observation),
        });
        defs.push(HeaderFieldDef {
            name: obs.name.to_string(),
            field_type: wire_type(obs.ty),
            subsystem: Some(obs.subsystem.to_string()),
            layer: Some("timing".to_string()),
            nullable: false,
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
