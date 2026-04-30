//! End-to-end test: missingno declares an extension field via a Profile,
//! Tracer captures it, the resulting trace round-trips with the extension
//! metadata in the header.

#![cfg(feature = "gbtrace")]

use std::collections::BTreeMap;

use gbtrace::format::read::GbtraceStore;
use gbtrace::header::Trigger;
use gbtrace::profile::FieldType;
use gbtrace::store::TraceStore;
use missingno_gb::trace::{BootRom, Profile, Tracer};
use missingno_gb::{GameBoy, cartridge::Cartridge};

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("missingno-ext-test-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &std::path::Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn minimal_rom() -> Vec<u8> {
    // 32 KiB ROM: NOP loop at 0x0100. Header bytes (logo, etc.) are not
    // checked at this layer — we use a `null` cartridge type.
    let mut rom = vec![0u8; 0x8000];
    rom[0x0100] = 0x00; // NOP
    rom[0x0101] = 0x18; // JR -2 → 0x0100
    rom[0x0102] = 0xFE;
    rom
}

#[test]
fn missingno_extension_field_roundtrip() {
    let dir = TempDir::new();
    let path = dir.path().join("ext.gbtrace");

    let mut extensions = BTreeMap::new();
    extensions.insert(
        "missingno".to_string(),
        vec!["pending_vector_resolve".to_string(), "halt_bug".to_string()],
    );

    let profile = Profile {
        name: "ext_test".into(),
        description: "extension-fields smoke test".into(),
        trigger: Trigger::Instruction,
        fields: vec!["pc".into(), "a".into()],
        memory: BTreeMap::new(),
        extensions,
    };

    let cartridge = Cartridge::new(minimal_rom(), None);
    let mut gb = GameBoy::new(cartridge, None);

    {
        let mut tracer = Tracer::create(&path, &profile, &gb, BootRom::Skip).unwrap();
        for _ in 0..16 {
            tracer.capture(&gb).unwrap();
            let _ = gb.step();
        }
        tracer.finish().unwrap();
    }

    let data = std::fs::read(&path).unwrap();
    let store = GbtraceStore::from_bytes(&data).unwrap();

    let hdr = store.header();
    assert_eq!(hdr.emulator, "missingno");
    assert_eq!(hdr.extension_fields.len(), 2);

    let pvr = hdr.extension_fields.get("pending_vector_resolve").unwrap();
    assert_eq!(pvr.field_type, FieldType::Bool);
    assert_eq!(pvr.source.as_deref(), Some("missingno"));
    assert!(pvr.description.is_some());

    let hb = hdr.extension_fields.get("halt_bug").unwrap();
    assert_eq!(hb.field_type, FieldType::Bool);

    // Profile fields followed by extensions, all emitted in order
    assert_eq!(
        hdr.fields,
        vec!["pc", "a", "pending_vector_resolve", "halt_bug"]
    );

    // Columns round-trip with the right types
    let pvr_col = hdr.fields.iter().position(|f| f == "pending_vector_resolve").unwrap();
    let hb_col = hdr.fields.iter().position(|f| f == "halt_bug").unwrap();
    for i in 0..store.entry_count() {
        let _ = store.get_bool(pvr_col, i);
        let _ = store.get_bool(hb_col, i);
    }

    assert!(store.entry_count() > 0);
}

#[test]
fn unknown_extension_name_is_rejected() {
    let dir = TempDir::new();
    let path = dir.path().join("bad.gbtrace");

    let mut extensions = BTreeMap::new();
    extensions.insert(
        "missingno".to_string(),
        vec!["this_field_does_not_exist".to_string()],
    );

    let profile = Profile {
        name: "bad_test".into(),
        description: String::new(),
        trigger: Trigger::Instruction,
        fields: vec!["pc".into()],
        memory: BTreeMap::new(),
        extensions,
    };

    let cartridge = Cartridge::new(minimal_rom(), None);
    let gb = GameBoy::new(cartridge, None);

    let result = Tracer::create(&path, &profile, &gb, BootRom::Skip);
    assert!(result.is_err(), "tracer should reject unknown extension name");
}
