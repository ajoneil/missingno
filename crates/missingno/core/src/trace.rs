//! The trace container's column vocabulary: how a console's hardware state
//! schema becomes the columns of a `.morepork` execution trace.
//!
//! A capture bridge belongs to its console — what it observes per step, and
//! where those observations come from, is that machine's business. What every
//! bridge shares is the *plan*: which schema fields a scope admits, in what
//! order, with which wire type and header metadata, and how a schema value
//! reaches the wire. That is the container's own vocabulary, so it lives here
//! with the schema rather than being re-authored per core.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::{HeaderFieldDef, PixFormat, TraceHeader};
use morepork::profile::FieldType as WireType;

pub use morepork::{BootRom, Error, Profile, Trigger};

use crate::state::{FieldType, PixelFormat, Provenance, StateValue, SystemStateSchema, Tier};

/// Which tier of the schema a trace captures. The observable surface is the
/// cross-emulator comparison ground; the full scope adds the boundary-complete
/// deep state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TraceScope {
    /// Tier-1 observable fields plus the trace observations. The diff surface.
    #[default]
    Observable,
    /// Also the schema's Tier-2a deep state.
    Full,
}

/// A trace-only observation: a per-step surface the state schema excludes
/// because it is not machine state. `T` is the producing core's own tag for it,
/// carried through the plan untouched.
pub struct ObservationDef<T: 'static> {
    pub name: &'static str,
    pub ty: FieldType,
    pub subsystem: &'static str,
    pub layer: &'static str,
    pub nullable: bool,
    pub observation: T,
}

/// How a planned column's value is produced at capture time.
pub enum Source<T> {
    /// A schema field, read from the per-capture record by name.
    Field(&'static str),
    /// One of the bridge's own observations.
    Observation(T),
}

/// One planned column: the type fixing its emitted width, and where its value
/// comes from.
pub struct Column<T> {
    pub ty: FieldType,
    pub nullable: bool,
    pub source: Source<T>,
}

pub fn wire_type(ty: FieldType) -> WireType {
    match ty {
        FieldType::Bool => WireType::Bool,
        FieldType::U8 => WireType::UInt8,
        FieldType::U16 => WireType::UInt16,
        // The wire format has no 32-bit column; a u32 field widens to u64.
        FieldType::U32 => WireType::UInt64,
        FieldType::Str => WireType::Str,
    }
}

pub fn tier_layer(tier: Tier) -> &'static str {
    match tier {
        Tier::Observable => "registers",
        Tier::Boundary => "internal",
        Tier::Tick => "timing",
    }
}

pub fn pix_format(format: PixelFormat) -> PixFormat {
    match format {
        PixelFormat::Shade2 => PixFormat::Shade2,
        PixelFormat::Rgb555 => PixFormat::Rgb555,
        PixelFormat::Indexed8 => PixFormat::Indexed8,
    }
}

/// Build the ordered column plan and matching header field defs for a scope.
/// Tier-1 (and, under `Full`, Tier-2a) schema fields lead; the observations
/// follow. Every column's type metadata comes from the schema or the
/// observation table — never re-authored by the bridge.
pub fn build_columns<T: Copy>(
    schema: &SystemStateSchema,
    scope: TraceScope,
    observations: &'static [ObservationDef<T>],
) -> (Vec<Column<T>>, Vec<HeaderFieldDef>) {
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

    for obs in observations {
        columns.push(Column {
            ty: obs.ty,
            nullable: obs.nullable,
            source: Source::Observation(obs.observation),
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

/// The header a bridge writes: the plan's field defs plus the identity of what
/// produced them.
pub struct TraceIdentity<'a> {
    pub rom_sha256: String,
    pub system: &'a str,
    pub isa: &'a str,
    pub model: &'a str,
    pub scope: TraceScope,
    pub trigger: Trigger,
    pub pix_format: PixFormat,
    pub boot_rom: BootRom,
    pub instruction_addr_field: &'a str,
    pub snapshot_kinds: Vec<String>,
}

/// Open a writer for a planned trace. The bridge owns what its columns mean;
/// the header shape around them is the container's.
pub fn create_writer(
    path: impl AsRef<Path>,
    identity: TraceIdentity,
    field_defs: Vec<HeaderFieldDef>,
) -> Result<MoreporkWriter, Error> {
    let header = TraceHeader {
        _header: true,
        format_version: "0.1.0".into(),
        emulator: "missingno".into(),
        emulator_version: env!("CARGO_PKG_VERSION").into(),
        rom_sha256: identity.rom_sha256,
        system: identity.system.into(),
        isa: identity.isa.into(),
        model: identity.model.into(),
        boot_rom: identity.boot_rom,
        profile: match identity.scope {
            TraceScope::Observable => "observable".into(),
            TraceScope::Full => "full".into(),
        },
        fields: field_defs.iter().map(|d| d.name.clone()).collect(),
        trigger: identity.trigger,
        pix_format: identity.pix_format,
        field_defs,
        instruction_addr_field: Some(identity.instruction_addr_field.into()),
        snapshot_kinds: identity.snapshot_kinds,
        notes: String::new(),
        ..Default::default()
    };
    MoreporkWriter::create(path, &header, &[])
}

/// Emit one schema value into its column, or the type-appropriate absence: a
/// null where the schema allows one, a zero where it does not (so the columns
/// stay aligned whatever the record carried).
pub fn emit_value(
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
