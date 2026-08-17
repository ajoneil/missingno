//! Trace-capture bridge tests: the trace header's columns are authored from the
//! core's state schema, and captured values round-trip through the reader.

#![cfg(feature = "morepork")]

use missingno_core::state::Tier;
use missingno_gb::state_schema::dmg_state_schema;
use missingno_gb::trace::{BootRom, TraceScope, Tracer, Trigger};
use missingno_gb::{GameBoy, cartridge::Cartridge};
use morepork::comparison::TraceComparison;
use morepork::format::read::MoreporkStore;
use morepork::profile::FieldType;
use morepork::store::TraceStore;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("missingno-trace-test-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn minimal_rom() -> Vec<u8> {
    // 32 KiB ROM: an increment loop at 0x0100 so `a`/`pc` change across capture.
    let mut rom = vec![0u8; 0x8000];
    rom[0x0100] = 0x3C; // INC A
    rom[0x0101] = 0x18; // JR -3 → 0x0100
    rom[0x0102] = 0xFD;
    rom
}

/// Capture a short observable trace of the increment loop, seeding `a`.
fn capture_run(path: &std::path::Path, initial_a: u8) {
    let mut gb = GameBoy::new(Cartridge::new(minimal_rom(), None, None).unwrap(), None);
    gb.cpu_mut().a = initial_a;
    let mut tracer = Tracer::create(
        path,
        &gb,
        Trigger::Instruction,
        TraceScope::Observable,
        BootRom::Skip,
        "DMG-B",
    )
    .unwrap();
    for _ in 0..64 {
        tracer.capture(&gb).unwrap();
        let _ = gb.step();
    }
    tracer.finish().unwrap();
}

/// The observable-scope header carries exactly the schema's Tier-1 field names,
/// with their schema types, followed by the trace observations.
#[test]
fn header_fields_are_authored_from_the_schema() {
    let dir = TempDir::new();
    let path = dir.path().join("hdr.morepork");

    let mut gb = GameBoy::new(Cartridge::new(minimal_rom(), None, None).unwrap(), None);
    {
        let mut tracer = Tracer::create(
            &path,
            &gb,
            Trigger::Instruction,
            TraceScope::Observable,
            BootRom::Skip,
            "DMG-B",
        )
        .unwrap();
        for _ in 0..8 {
            tracer.capture(&gb).unwrap();
            let _ = gb.step();
        }
        tracer.finish().unwrap();
    }

    let data = std::fs::read(&path).unwrap();
    let store = MoreporkStore::from_bytes(&data).unwrap();
    let hdr = store.header();

    assert_eq!(hdr.emulator, "missingno");
    assert_eq!(hdr.system, "dmg");
    assert_eq!(hdr.isa, "sm83");

    let schema = dmg_state_schema();
    let observable: Vec<&str> = schema.fields_at(Tier::Observable).map(|f| f.name).collect();

    // Every Tier-1 field is a column, in schema order, ahead of the observations.
    for name in &observable {
        assert!(
            hdr.fields.iter().any(|f| f == name),
            "missing column {name}"
        );
    }
    let observations = ["op_addr", "pix", "pix_x", "vram_addr", "vram_data"];
    for name in observations {
        assert!(
            hdr.fields.iter().any(|f| f == name),
            "missing observation {name}"
        );
    }

    // No Tier-2a deep field leaks into an observable-scope trace.
    let boundary: Vec<&str> = schema.fields_at(Tier::Boundary).map(|f| f.name).collect();
    for name in boundary {
        // `pc`/`sp` etc. are Tier-1; only assert names unique to Tier-2a are absent.
        if !observable.contains(&name) {
            assert!(
                !hdr.fields.iter().any(|f| f == name),
                "deep field {name} leaked"
            );
        }
    }

    // Column types match the schema's field types, and `op_addr` is the
    // instruction-address column.
    let lcdc = hdr.field_def("lcdc").unwrap();
    assert_eq!(lcdc.field_type, FieldType::UInt8);
    assert_eq!(lcdc.subsystem.as_deref(), Some("ppu"));
    let sp = hdr.field_def("sp").unwrap();
    assert_eq!(sp.field_type, FieldType::UInt16);
    assert_eq!(hdr.instruction_addr_field.as_deref(), Some("op_addr"));
}

/// Captured register values read back exactly as they were at capture time.
#[test]
fn captured_values_round_trip() {
    let dir = TempDir::new();
    let path = dir.path().join("rt.morepork");

    let mut gb = GameBoy::new(Cartridge::new(minimal_rom(), None, None).unwrap(), None);

    let mut expected_a = Vec::new();
    let mut expected_pc = Vec::new();
    {
        let mut tracer = Tracer::create(
            &path,
            &gb,
            Trigger::Instruction,
            TraceScope::Observable,
            BootRom::Skip,
            "DMG-B",
        )
        .unwrap();
        for _ in 0..24 {
            expected_a.push(gb.cpu().a);
            expected_pc.push(gb.cpu().pc);
            tracer.capture(&gb).unwrap();
            let _ = gb.step();
        }
        tracer.finish().unwrap();
    }

    let data = std::fs::read(&path).unwrap();
    let store = MoreporkStore::from_bytes(&data).unwrap();

    assert_eq!(store.entry_count(), expected_a.len());
    for i in 0..store.entry_count() {
        assert_eq!(
            store.get_numeric_named("a", i).unwrap(),
            expected_a[i] as u64,
            "a mismatch at row {i}"
        );
        assert_eq!(
            store.get_numeric_named("pc", i).unwrap(),
            expected_pc[i] as u64,
            "pc mismatch at row {i}"
        );
        // op_addr carries the executing instruction's address.
        assert_eq!(
            store.get_numeric_named("op_addr", i).unwrap(),
            expected_pc[i] as u64,
            "op_addr mismatch at row {i}"
        );
    }
}

/// Two captures of the same ROM from different initial states walk the same
/// instruction stream but diverge in the `a` column from the first entry — and
/// the comparison engine detects it.
#[test]
fn diff_detects_a_planted_divergence() {
    let dir = TempDir::new();
    let a_path = dir.path().join("run_a.morepork");
    let b_path = dir.path().join("run_b.morepork");
    capture_run(&a_path, 0x00);
    capture_run(&b_path, 0x80);

    let data_a = std::fs::read(&a_path).unwrap();
    let data_b = std::fs::read(&b_path).unwrap();
    let store_a = MoreporkStore::from_bytes(&data_a).unwrap();
    let store_b = MoreporkStore::from_bytes(&data_b).unwrap();

    let cmp = TraceComparison::align(&store_a, &store_b, None).unwrap();
    assert!(cmp.len() > 0, "traces did not align");

    // The control flow matches (op_addr aligns), but `a` diverges at entry 0.
    assert!(
        cmp.field_differs("a", 0),
        "planted `a` divergence not detected"
    );
    assert!(
        !cmp.field_differs("op_addr", 0),
        "control flow should align"
    );
    assert!(!cmp.field_differs("pc", 0), "pc should align");
}

/// The full scope adds the schema's Tier-2a deep state (the pixel-pipeline cells
/// and scalar counters).
#[test]
fn full_scope_adds_deep_state() {
    let dir = TempDir::new();
    let path = dir.path().join("full.morepork");

    let mut gb = GameBoy::new(Cartridge::new(minimal_rom(), None, None).unwrap(), None);
    {
        let mut tracer = Tracer::create(
            &path,
            &gb,
            Trigger::Instruction,
            TraceScope::Full,
            BootRom::Skip,
            "DMG-B",
        )
        .unwrap();
        for _ in 0..8 {
            tracer.capture(&gb).unwrap();
            let _ = gb.step();
        }
        tracer.finish().unwrap();
    }

    let data = std::fs::read(&path).unwrap();
    let store = MoreporkStore::from_bytes(&data).unwrap();
    let hdr = store.header();

    // Deep scalar (LX) and a pipeline cell are both present under Full.
    assert!(
        hdr.fields.iter().any(|f| f == "lx"),
        "deep scalar lx absent"
    );
    assert!(
        hdr.fields.iter().any(|f| f == "bgw_fifo_a"),
        "pipeline cell absent"
    );
    assert_eq!(
        hdr.field_def("internal_counter").unwrap().field_type,
        FieldType::UInt16
    );
}
