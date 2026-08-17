//! A machine's state described as hardware-named fields — DATA an authored
//! save-state bridge or trace writer walks. Records are keyed on the hardware's
//! own register/counter/latch names, never an emulator's internal layout, so a
//! second emulator can produce and consume a record by knowing the silicon.
//!
//! This is the state counterpart to [`crate::inspect`]'s presentation
//! vocabulary: the same hardware names with the display concerns (tones,
//! hover help, section layout) stripped off. A core describes its state once,
//! here, and both framings — a save state (every field at one instant) and a
//! trace (a per-step subset of fields over time) — key off the same schema.

use std::collections::BTreeMap;

/// The native type of a state field's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Bool,
    U8,
    U16,
    U32,
    /// A short identifier string (a cartridge mapper's type name).
    Str,
}

/// How observable a field is — how many emulators can produce it, and which
/// fidelity of save state it serves. The tiers are ordered: a producer filling
/// a tier fills the tiers below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Tier 1 — the CPU-visible surface: registers, the IO register file,
    /// memory, the framebuffer. Any emulator can produce and consume these by
    /// knowing the hardware; cross-emulator comparison operates here.
    Observable,
    /// Tier 2a — deep hardware state named after the silicon (pipeline latches,
    /// channel counters, the frame-sequencer step). Enough to restore
    /// bit-exactly at an instruction/frame boundary. Fewer emulators fill it; a
    /// gate-level one fills nearly all of it.
    Boundary,
    /// Tier 2b — the tick-complete residue: the CPU micro-sequencer phase, bus
    /// latches, and the exact clock edge. Legitimate hardware state, but not yet
    /// named at the seam — no field carries this tier until that naming lands.
    /// It is the honest home of arbitrary-tick restore.
    Tick,
}

/// Where a field's definition comes from, so a hardware-canonical name is
/// distinguishable from an emulator-private probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// A hardware quantity with a name any conforming emulator can produce — a
    /// CPU-visible register, or a deep signal named after its die gate.
    Hardware,
    /// A probe private to one emulator, with no hardware-canonical name yet.
    Emulator(&'static str),
}

/// One field of a machine's state: its hardware name, native type, tier, the
/// subsystem it belongs to, and its provenance. `help` is a one-line
/// description; for a gate-named deep field it names the concordance gate.
#[derive(Clone, Copy, Debug)]
pub struct FieldDef {
    pub name: &'static str,
    pub ty: FieldType,
    pub tier: Tier,
    /// The hardware block this field belongs to (`"cpu"`, `"ppu"`, `"apu"`, …).
    pub subsystem: &'static str,
    pub provenance: Provenance,
    /// A producer may omit the field (a mapper lacking this latch, an emulator
    /// not modelling this signal); a record then carries it as absent.
    pub nullable: bool,
    pub help: Option<&'static str>,
}

impl FieldDef {
    /// A Tier-1 observable hardware field.
    pub const fn observable(name: &'static str, ty: FieldType, subsystem: &'static str) -> Self {
        FieldDef {
            name,
            ty,
            tier: Tier::Observable,
            subsystem,
            provenance: Provenance::Hardware,
            nullable: false,
            help: None,
        }
    }

    /// A Tier-2a boundary-complete deep hardware field.
    pub const fn boundary(name: &'static str, ty: FieldType, subsystem: &'static str) -> Self {
        FieldDef {
            name,
            ty,
            tier: Tier::Boundary,
            subsystem,
            provenance: Provenance::Hardware,
            nullable: false,
            help: None,
        }
    }

    /// Attach a one-line description (the concordance gate name, for deep state).
    pub const fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
    }

    /// Mark the field omittable by a producer that can't supply it.
    pub const fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    /// Attribute the field to one emulator rather than to canonical hardware.
    pub const fn sourced(mut self, emulator: &'static str) -> Self {
        self.provenance = Provenance::Emulator(emulator);
        self
    }
}

/// A named byte region of a machine's state. An addressable region carries its
/// CPU bus start; an off-bus region (palette RAM reached through an index port)
/// carries `None` and is named only.
#[derive(Clone, Copy, Debug)]
pub struct MemorySpan {
    pub name: &'static str,
    /// The region's CPU bus start, or `None` for state off the CPU map.
    pub start: Option<u32>,
    pub len: u32,
    /// Present only when the media provides it (external cartridge RAM); a
    /// record omits an absent optional span.
    pub optional: bool,
    pub help: Option<&'static str>,
}

impl MemorySpan {
    /// A region mapped into the CPU address space at `start`.
    pub const fn addressable(name: &'static str, start: u32, len: u32) -> Self {
        MemorySpan {
            name,
            start: Some(start),
            len,
            optional: false,
            help: None,
        }
    }

    /// A region with no CPU address (palette RAM, reached through an index port).
    pub const fn off_bus(name: &'static str, len: u32) -> Self {
        MemorySpan {
            name,
            start: None,
            len,
            optional: false,
            help: None,
        }
    }

    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub const fn help(mut self, help: &'static str) -> Self {
        self.help = Some(help);
        self
    }
}

/// How a system's framebuffer encodes its pixels — the pre-resolution domain
/// the accuracy references compare in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 2-bit shade indices (DMG).
    Shade2,
    /// 15-bit RGB555 words (CGB/AGB).
    Rgb555,
    /// Indices into the frame's own palette (emergent-palette systems).
    Indexed8,
}

/// The framebuffer a system produces.
#[derive(Clone, Copy, Debug)]
pub struct FrameSpec {
    pub width: u32,
    /// Fixed frame height, or `None` for an emergent-sync display whose line
    /// count varies per field.
    pub height: Option<u32>,
    pub format: PixelFormat,
}

/// A whole system's authored state schema: its hardware-named fields, its
/// memory regions, and its framebuffer. Keyed by `system`, this is the only
/// thing a record — save state or trace — keys its state on.
pub struct SystemStateSchema {
    /// The system this schema describes (`"dmg"`, `"cgb"`). Keys a trace
    /// header and a save state's compatibility check.
    pub system: &'static str,
    pub fields: Vec<FieldDef>,
    pub memory: Vec<MemorySpan>,
    pub frame: FrameSpec,
}

impl SystemStateSchema {
    /// The field of the given hardware name, if the schema defines it.
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// The memory span of the given name, if the schema defines it.
    pub fn span(&self, name: &str) -> Option<&MemorySpan> {
        self.memory.iter().find(|s| s.name == name)
    }

    /// The fields at one tier, in schema order.
    pub fn fields_at(&self, tier: Tier) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().filter(move |f| f.tier == tier)
    }

    /// Rebuild a validated [`StateRecord`] from name/value pairs read off a save
    /// file. Each name is resolved to this schema's own static field name (a
    /// name the schema does not define is rejected), then the assembled record
    /// is validated — so a well-formed record over the wrong schema fails here
    /// with a concrete field error rather than silently mis-restoring.
    pub fn record_from(
        &self,
        fields: impl IntoIterator<Item = (String, StateValue)>,
    ) -> Result<StateRecord, RecordError> {
        let mut record = StateRecord::new();
        for (name, value) in fields {
            match self.field(&name) {
                Some(field) => {
                    record.set(field.name, value);
                }
                None => return Err(RecordError::UnknownField(name)),
            }
        }
        record.validate(self)?;
        Ok(record)
    }

    /// Check the schema is well-formed: unique field names, unique span names,
    /// and addressable spans that neither overflow the address space nor
    /// overlap one another.
    pub fn check(&self) -> Result<(), SchemaError> {
        let mut seen_fields = BTreeMap::new();
        for field in &self.fields {
            if seen_fields.insert(field.name, ()).is_some() {
                return Err(SchemaError::DuplicateField(field.name));
            }
        }

        let mut seen_spans = BTreeMap::new();
        for span in &self.memory {
            if seen_spans.insert(span.name, ()).is_some() {
                return Err(SchemaError::DuplicateSpan(span.name));
            }
            if let Some(start) = span.start
                && start.checked_add(span.len).is_none()
            {
                return Err(SchemaError::SpanOverflow(span.name));
            }
        }

        for (i, a) in self.memory.iter().enumerate() {
            let (Some(a_start), a_len) = (a.start, a.len) else {
                continue;
            };
            for b in &self.memory[i + 1..] {
                let Some(b_start) = b.start else { continue };
                let overlaps = a_start < b_start + b.len && b_start < a_start + a_len;
                if overlaps {
                    return Err(SchemaError::OverlappingSpans(a.name, b.name));
                }
            }
        }

        Ok(())
    }
}

/// Why a schema is malformed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    DuplicateField(&'static str),
    DuplicateSpan(&'static str),
    OverlappingSpans(&'static str, &'static str),
    SpanOverflow(&'static str),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::DuplicateField(name) => write!(f, "duplicate field name '{name}'"),
            SchemaError::DuplicateSpan(name) => write!(f, "duplicate memory span '{name}'"),
            SchemaError::OverlappingSpans(a, b) => {
                write!(f, "memory spans '{a}' and '{b}' overlap")
            }
            SchemaError::SpanOverflow(name) => {
                write!(f, "memory span '{name}' runs past the address space")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

/// One field's value in a [`StateRecord`]. Fixed-width integers share the
/// [`StateValue::Int`] carrier; the field's [`FieldType`] fixes the width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    Bool(bool),
    Int(u32),
    Text(String),
    /// A nullable field a producer omitted.
    Null,
}

impl StateValue {
    /// Whether this value fits a field of the given type.
    pub fn matches(&self, ty: FieldType) -> bool {
        match (self, ty) {
            (StateValue::Bool(_), FieldType::Bool)
            | (StateValue::Text(_), FieldType::Str)
            | (StateValue::Null, _) => true,
            (StateValue::Int(v), FieldType::U8) => *v <= u8::MAX as u32,
            (StateValue::Int(v), FieldType::U16) => *v <= u16::MAX as u32,
            (StateValue::Int(_), FieldType::U32) => true,
            _ => false,
        }
    }
}

impl From<bool> for StateValue {
    fn from(value: bool) -> Self {
        StateValue::Bool(value)
    }
}
impl From<u8> for StateValue {
    fn from(value: u8) -> Self {
        StateValue::Int(value as u32)
    }
}
impl From<u16> for StateValue {
    fn from(value: u16) -> Self {
        StateValue::Int(value as u32)
    }
}
impl From<u32> for StateValue {
    fn from(value: u32) -> Self {
        StateValue::Int(value)
    }
}
impl From<&str> for StateValue {
    fn from(value: &str) -> Self {
        StateValue::Text(value.to_owned())
    }
}
impl From<String> for StateValue {
    fn from(value: String) -> Self {
        StateValue::Text(value)
    }
}

/// A machine's scalar state at one instant, keyed by hardware field name — the
/// in-memory form the bridge fills (capture) and the format engine walks
/// (serialise). Memory regions and the framebuffer travel as separate blobs,
/// not through this record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateRecord {
    values: BTreeMap<&'static str, StateValue>,
}

impl StateRecord {
    pub fn new() -> Self {
        StateRecord::default()
    }

    /// Record one field's value.
    pub fn set(&mut self, name: &'static str, value: impl Into<StateValue>) -> &mut Self {
        self.values.insert(name, value.into());
        self
    }

    pub fn get(&self, name: &str) -> Option<&StateValue> {
        self.values.get(name)
    }

    /// The recorded fields, in name order, for a serializer to walk.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &StateValue)> {
        self.values.iter().map(|(&name, value)| (name, value))
    }

    /// Check the record against a schema: every non-nullable field present with
    /// a matching type, every present value well-typed, and no field of a name
    /// the schema does not define.
    pub fn validate(&self, schema: &SystemStateSchema) -> Result<(), RecordError> {
        for field in &schema.fields {
            match self.values.get(field.name) {
                None => {
                    if !field.nullable {
                        return Err(RecordError::MissingField(field.name));
                    }
                }
                Some(value) => {
                    if !value.matches(field.ty) {
                        return Err(RecordError::TypeMismatch(field.name));
                    }
                }
            }
        }
        for name in self.values.keys() {
            if schema.field(name).is_none() {
                return Err(RecordError::UnknownField((*name).to_owned()));
            }
        }
        Ok(())
    }
}

/// Why a record fails to match its schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordError {
    MissingField(&'static str),
    TypeMismatch(&'static str),
    UnknownField(String),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::MissingField(name) => write!(f, "record is missing field '{name}'"),
            RecordError::TypeMismatch(name) => {
                write!(f, "record field '{name}' has the wrong type")
            }
            RecordError::UnknownField(name) => {
                write!(f, "record field '{name}' is not in the schema")
            }
        }
    }
}

impl std::error::Error for RecordError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_schema() -> SystemStateSchema {
        SystemStateSchema {
            system: "test",
            fields: vec![
                FieldDef::observable("a", FieldType::U8, "cpu"),
                FieldDef::observable("pc", FieldType::U16, "cpu"),
                FieldDef::boundary("lx", FieldType::U8, "ppu").help("LX counter"),
                FieldDef::boundary("mbc_type", FieldType::Str, "cartridge").nullable(),
            ],
            memory: vec![
                MemorySpan::addressable("vram", 0x8000, 0x2000),
                MemorySpan::addressable("wram", 0xC000, 0x2000),
                MemorySpan::off_bus("cram", 64),
            ],
            frame: FrameSpec {
                width: 160,
                height: Some(144),
                format: PixelFormat::Shade2,
            },
        }
    }

    #[test]
    fn tiers_order_observable_below_deep() {
        assert!(Tier::Observable < Tier::Boundary);
        assert!(Tier::Boundary < Tier::Tick);
    }

    #[test]
    fn lookup_and_tier_filter() {
        let schema = tiny_schema();
        assert_eq!(schema.field("pc").unwrap().ty, FieldType::U16);
        assert!(schema.field("missing").is_none());
        let observable: Vec<_> = schema.fields_at(Tier::Observable).map(|f| f.name).collect();
        assert_eq!(observable, ["a", "pc"]);
        let boundary: Vec<_> = schema.fields_at(Tier::Boundary).map(|f| f.name).collect();
        assert_eq!(boundary, ["lx", "mbc_type"]);
    }

    #[test]
    fn well_formed_schema_checks() {
        assert_eq!(tiny_schema().check(), Ok(()));
    }

    #[test]
    fn check_rejects_duplicate_field() {
        let mut schema = tiny_schema();
        schema
            .fields
            .push(FieldDef::observable("a", FieldType::U8, "cpu"));
        assert_eq!(schema.check(), Err(SchemaError::DuplicateField("a")));
    }

    #[test]
    fn check_rejects_overlapping_spans() {
        let mut schema = tiny_schema();
        schema
            .memory
            .push(MemorySpan::addressable("overlap", 0x9000, 0x1000));
        assert!(matches!(
            schema.check(),
            Err(SchemaError::OverlappingSpans(_, _))
        ));
    }

    #[test]
    fn value_type_matching() {
        assert!(StateValue::from(0xFFu8).matches(FieldType::U8));
        assert!(!StateValue::Int(0x100).matches(FieldType::U8));
        assert!(StateValue::Int(0x100).matches(FieldType::U16));
        assert!(StateValue::from(true).matches(FieldType::Bool));
        assert!(StateValue::from("mbc1").matches(FieldType::Str));
        assert!(StateValue::Null.matches(FieldType::U32));
        assert!(!StateValue::from(true).matches(FieldType::U8));
    }

    #[test]
    fn record_validates_against_schema() {
        let schema = tiny_schema();
        let mut record = StateRecord::new();
        record.set("a", 0x12u8).set("pc", 0x0150u16).set("lx", 40u8);
        // `mbc_type` is nullable and omitted — still valid.
        assert_eq!(record.validate(&schema), Ok(()));
    }

    #[test]
    fn record_rejects_missing_required_field() {
        let schema = tiny_schema();
        let mut record = StateRecord::new();
        record.set("a", 0x12u8).set("lx", 40u8);
        assert_eq!(
            record.validate(&schema),
            Err(RecordError::MissingField("pc"))
        );
    }

    #[test]
    fn record_rejects_unknown_and_mistyped_fields() {
        let schema = tiny_schema();
        let mut record = StateRecord::new();
        record.set("a", 0x12u8).set("pc", 0x0150u16).set("lx", 40u8);

        let mut mistyped = record.clone();
        mistyped.set("a", true);
        assert_eq!(
            mistyped.validate(&schema),
            Err(RecordError::TypeMismatch("a"))
        );

        let mut unknown = record.clone();
        unknown.set("ghost", 1u8);
        assert!(matches!(
            unknown.validate(&schema),
            Err(RecordError::UnknownField(_))
        ));
    }
}
