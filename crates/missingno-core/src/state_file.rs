//! The state-file framing: a full-state save file over the hardware-named
//! [`SystemStateSchema`]. A save state is one full-state record — every schema
//! field at one instant, plus its memory spans and a framebuffer — in a
//! self-describing container. Version 3, no migration: a reader rejects any
//! other version and the state is regenerated (the effort-wide breaking
//! posture).
//!
//! The container is deliberately lean — a hand-rolled little-endian layout with
//! no compression — so a core serializes and restores a save state without the
//! trace container's Arrow/zstd chunk machinery, which a save state (a single
//! record, no chunk stream) does not need. The bytes are self-describing: they
//! carry the system id, the field names and their values, and the memory span
//! names, so a reader validates a record against a schema without prior
//! knowledge of the producer.

use crate::state::{FieldType, PixelFormat, StateRecord, StateValue};

/// Magic bytes of a state save file. Distinct from the trace container's `MPRK`
/// so the two framings are never confused.
pub const STATE_MAGIC: &[u8; 4] = b"MPSV";

/// State-file container version. A reader rejects any other value outright.
pub const STATE_VERSION: u8 = 3;

/// The framebuffer a save file carries — informational, so a viewer can show a
/// state's screenshot; a core regenerates the display from its restored state.
#[derive(Clone, Debug, PartialEq)]
pub struct StateFrame {
    pub width: u32,
    pub height: Option<u32>,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

/// The producer-identifying metadata a save file carries, so `load` can refuse
/// a state from an incompatible producer or a different ROM.
pub struct StateMeta<'a> {
    pub system: &'a str,
    pub rom_sha256: Option<&'a str>,
    pub emulator: &'a str,
    pub emulator_version: &'a str,
}

/// A parsed save file: its identity, the raw name/value field pairs (rebuilt
/// into a validated [`StateRecord`] against a schema by the consumer), its
/// memory spans, and its framebuffer.
#[derive(Clone, Debug, PartialEq)]
pub struct StateFile {
    pub system: String,
    pub rom_sha256: Option<String>,
    pub emulator: String,
    pub emulator_version: String,
    pub fields: Vec<(String, StateValue)>,
    pub memory: Vec<(String, Vec<u8>)>,
    pub frame: Option<StateFrame>,
}

/// Why a save file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateFileError {
    /// The leading bytes are not a state save file.
    BadMagic,
    /// The container version is not the one this build implements.
    UnsupportedVersion(u8),
    /// The data ended before a declared field or span was complete.
    Truncated,
    /// A string field was not valid UTF-8.
    BadUtf8,
    /// A field's type or value tag was not a known code.
    BadEncoding,
}

impl std::fmt::Display for StateFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateFileError::BadMagic => f.write_str("not a state save file"),
            StateFileError::UnsupportedVersion(v) => {
                write!(f, "unsupported state-file version {v} (regenerate)")
            }
            StateFileError::Truncated => f.write_str("state save file is truncated"),
            StateFileError::BadUtf8 => f.write_str("state save file has invalid text"),
            StateFileError::BadEncoding => f.write_str("state save file has an unknown encoding"),
        }
    }
}

impl std::error::Error for StateFileError {}

fn field_type_code(ty: FieldType) -> u8 {
    match ty {
        FieldType::Bool => 0,
        FieldType::U8 => 1,
        FieldType::U16 => 2,
        FieldType::U32 => 3,
        FieldType::Str => 4,
    }
}

fn pixel_format_code(format: PixelFormat) -> u8 {
    match format {
        PixelFormat::Shade2 => 0,
        PixelFormat::Rgb555 => 1,
        PixelFormat::Indexed8 => 2,
    }
}

fn pixel_format_from_code(code: u8) -> Result<PixelFormat, StateFileError> {
    match code {
        0 => Ok(PixelFormat::Shade2),
        1 => Ok(PixelFormat::Rgb555),
        2 => Ok(PixelFormat::Indexed8),
        _ => Err(StateFileError::BadEncoding),
    }
}

/// A value's type as it appears in a record, so the field type travels with the
/// value even though the schema also fixes it (self-description).
fn value_type_code(value: &StateValue) -> u8 {
    match value {
        StateValue::Bool(_) => field_type_code(FieldType::Bool),
        StateValue::Int(_) => field_type_code(FieldType::U32),
        StateValue::Text(_) => field_type_code(FieldType::Str),
        StateValue::Null => 0xFF,
    }
}

// ── Write ────────────────────────────────────────────────────────

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_value(out: &mut Vec<u8>, value: &StateValue) {
    match value {
        StateValue::Bool(b) => {
            out.push(0);
            out.push(*b as u8);
        }
        StateValue::Int(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        StateValue::Text(text) => {
            out.push(2);
            put_str(out, text);
        }
        StateValue::Null => out.push(3),
    }
}

/// Serialize a full-state record, its memory spans, and its framebuffer into a
/// save file. Memory spans are written in the order given; the frame is
/// optional.
pub fn write_state_file(
    meta: &StateMeta,
    record: &StateRecord,
    memory: &[(&str, Vec<u8>)],
    frame: Option<&StateFrame>,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION);

    put_str(&mut out, meta.system);
    put_str(&mut out, meta.rom_sha256.unwrap_or(""));
    put_str(&mut out, meta.emulator);
    put_str(&mut out, meta.emulator_version);

    let fields: Vec<_> = record.iter().collect();
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for (name, value) in fields {
        put_str(&mut out, name);
        out.push(value_type_code(value));
        put_value(&mut out, value);
    }

    out.extend_from_slice(&(memory.len() as u32).to_le_bytes());
    for (name, bytes) in memory {
        put_str(&mut out, name);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    match frame {
        Some(frame) => {
            out.push(1);
            out.extend_from_slice(&frame.width.to_le_bytes());
            match frame.height {
                Some(height) => {
                    out.push(1);
                    out.extend_from_slice(&height.to_le_bytes());
                }
                None => out.push(0),
            }
            out.push(pixel_format_code(frame.format));
            out.extend_from_slice(&(frame.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&frame.data);
        }
        None => out.push(0),
    }

    out
}

// ── Read ─────────────────────────────────────────────────────────

/// A cursor over the save-file bytes with bounds-checked little-endian reads.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StateFileError> {
        let end = self.pos.checked_add(n).ok_or(StateFileError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(StateFileError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, StateFileError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, StateFileError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<String, StateFileError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| StateFileError::BadUtf8)
    }

    fn value(&mut self) -> Result<StateValue, StateFileError> {
        match self.u8()? {
            0 => Ok(StateValue::Bool(self.u8()? != 0)),
            1 => Ok(StateValue::Int(self.u32()?)),
            2 => Ok(StateValue::Text(self.string()?)),
            3 => Ok(StateValue::Null),
            _ => Err(StateFileError::BadEncoding),
        }
    }
}

/// Parse a save file into its identity, fields, memory spans, and framebuffer.
/// Rebuilding the fields into a validated record is the consumer's step — it
/// holds the schema to validate against.
pub fn read_state_file(bytes: &[u8]) -> Result<StateFile, StateFileError> {
    let mut reader = Reader { bytes, pos: 0 };

    if reader.take(4)? != STATE_MAGIC {
        return Err(StateFileError::BadMagic);
    }
    let version = reader.u8()?;
    if version != STATE_VERSION {
        return Err(StateFileError::UnsupportedVersion(version));
    }

    let system = reader.string()?;
    let rom_sha256 = reader.string()?;
    let emulator = reader.string()?;
    let emulator_version = reader.string()?;

    let field_count = reader.u32()? as usize;
    let mut fields = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        let name = reader.string()?;
        let _type_code = reader.u8()?;
        let value = reader.value()?;
        fields.push((name, value));
    }

    let span_count = reader.u32()? as usize;
    let mut memory = Vec::with_capacity(span_count);
    for _ in 0..span_count {
        let name = reader.string()?;
        let len = reader.u32()? as usize;
        let data = reader.take(len)?.to_vec();
        memory.push((name, data));
    }

    let frame = if reader.u8()? == 1 {
        let width = reader.u32()?;
        let height = if reader.u8()? == 1 {
            Some(reader.u32()?)
        } else {
            None
        };
        let format = pixel_format_from_code(reader.u8()?)?;
        let len = reader.u32()? as usize;
        let data = reader.take(len)?.to_vec();
        Some(StateFrame {
            width,
            height,
            format,
            data,
        })
    } else {
        None
    };

    Ok(StateFile {
        system,
        rom_sha256: (!rom_sha256.is_empty()).then_some(rom_sha256),
        emulator,
        emulator_version,
        fields,
        memory,
        frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FieldDef, FieldType, FrameSpec, MemorySpan, SystemStateSchema};

    fn schema() -> SystemStateSchema {
        SystemStateSchema {
            system: "dmg",
            fields: vec![
                FieldDef::observable("a", FieldType::U8, "cpu"),
                FieldDef::observable("pc", FieldType::U16, "cpu"),
                FieldDef::boundary("mbc_type", FieldType::Str, "cartridge"),
                FieldDef::boundary("ime", FieldType::Bool, "cpu"),
            ],
            memory: vec![MemorySpan::addressable("wram", 0xC000, 0x2000)],
            frame: FrameSpec {
                width: 160,
                height: Some(144),
                format: PixelFormat::Shade2,
            },
        }
    }

    fn full_record() -> StateRecord {
        let mut record = StateRecord::new();
        record
            .set("a", 0x42u8)
            .set("pc", 0x0150u16)
            .set("mbc_type", "mbc1")
            .set("ime", true);
        record
    }

    #[test]
    fn round_trips_record_memory_and_frame() {
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: Some("abcd"),
            emulator: "missingno",
            emulator_version: "0.0.1",
        };
        let record = full_record();
        let memory = vec![("wram", vec![7u8; 0x2000])];
        let frame = StateFrame {
            width: 160,
            height: Some(144),
            format: PixelFormat::Shade2,
            data: vec![1u8; 160 * 144],
        };
        let bytes = write_state_file(&meta, &record, &memory, Some(&frame));

        let file = read_state_file(&bytes).unwrap();
        assert_eq!(file.system, "dmg");
        assert_eq!(file.rom_sha256.as_deref(), Some("abcd"));
        assert_eq!(file.emulator, "missingno");
        assert_eq!(file.memory.len(), 1);
        assert_eq!(file.memory[0].0, "wram");
        assert_eq!(file.memory[0].1.len(), 0x2000);
        assert_eq!(file.frame.as_ref().unwrap().data.len(), 160 * 144);

        let rebuilt = schema().record_from(file.fields).unwrap();
        assert_eq!(rebuilt.get("a"), Some(&StateValue::Int(0x42)));
        assert_eq!(rebuilt.get("pc"), Some(&StateValue::Int(0x0150)));
        assert_eq!(rebuilt.get("ime"), Some(&StateValue::Bool(true)));
    }

    #[test]
    fn round_trips_without_a_frame() {
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: None,
            emulator: "missingno",
            emulator_version: "0.0.1",
        };
        let bytes = write_state_file(&meta, &full_record(), &[], None);
        let file = read_state_file(&bytes).unwrap();
        assert!(file.rom_sha256.is_none());
        assert!(file.frame.is_none());
        assert!(file.memory.is_empty());
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(read_state_file(b"XXXX\x03"), Err(StateFileError::BadMagic));
    }

    #[test]
    fn rejects_unsupported_version() {
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: None,
            emulator: "e",
            emulator_version: "v",
        };
        let mut bytes = write_state_file(&meta, &full_record(), &[], None);
        bytes[4] = 99;
        assert_eq!(
            read_state_file(&bytes),
            Err(StateFileError::UnsupportedVersion(99))
        );
    }

    #[test]
    fn rejects_truncated() {
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: None,
            emulator: "e",
            emulator_version: "v",
        };
        let bytes = write_state_file(&meta, &full_record(), &[], None);
        assert_eq!(
            read_state_file(&bytes[..bytes.len() - 3]),
            Err(StateFileError::Truncated)
        );
    }

    #[test]
    fn wrong_schema_rejects_the_record() {
        // A record written for one schema, read back against a schema that has
        // an extra required field, fails to rebuild.
        let meta = StateMeta {
            system: "dmg",
            rom_sha256: None,
            emulator: "e",
            emulator_version: "v",
        };
        let bytes = write_state_file(&meta, &full_record(), &[], None);
        let file = read_state_file(&bytes).unwrap();

        let mut other = schema();
        other
            .fields
            .push(FieldDef::observable("sp", FieldType::U16, "cpu"));
        assert!(other.record_from(file.fields).is_err());
    }
}
