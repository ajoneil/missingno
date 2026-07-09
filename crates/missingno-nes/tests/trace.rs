//! End-to-end gbtrace capture: run a tiny NROM program, write a trace,
//! and read it back through gbtrace's own reader.
#![cfg(feature = "gbtrace")]

use missingno_nes::console::Nes;
use missingno_nes::trace::{Profile, Tracer, step_instruction_counted};

/// 32 KiB NROM: a counting loop at $8000, reset vector pointing at it.
fn test_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 16 + 32 * 1024];
    rom[0..4].copy_from_slice(b"NES\x1a");
    rom[4] = 2; // two 16 KiB PRG banks
    let prg = 16;
    let program = [
        0xa2, 0x00, // ldx #$00
        0xe8, // inx
        0x8e, 0x00, 0x02, // stx $0200
        0x4c, 0x02, 0x80, // jmp $8002
    ];
    rom[prg..prg + program.len()].copy_from_slice(&program);
    rom[prg + 0x7FFC] = 0x00; // reset vector -> $8000
    rom[prg + 0x7FFD] = 0x80;
    rom
}

#[test]
fn captures_a_readable_trace_with_frames() {
    let rom = test_rom();
    let mut nes = Nes::new(&rom).unwrap();

    let profile = Profile::parse(
        r#"
[profile]
name = "nes-test"
description = "capture test"
trigger = "instruction"
family = "nes"

[fields]
cpu = ["registers", "timing"]
ppu = "registers"

[fields.memory]
counter = "0200"
"#,
    )
    .unwrap();

    let path = std::env::temp_dir().join(format!(
        "missingno-nes-trace-test-{}.gbtrace",
        std::process::id()
    ));
    let mut tracer = Tracer::create(&path, &profile, &rom).unwrap();

    // ~2.5 frames of the counting loop (one frame ≈ 29781 CPU cycles).
    let mut total_cycles = 0u64;
    let mut last_cycles = 0u16;
    let mut frames = 0;
    while total_cycles < 75_000 {
        tracer.capture(&nes, last_cycles).unwrap();
        last_cycles = step_instruction_counted(&mut nes);
        total_cycles += last_cycles as u64;
        if let Some(frame) = nes.take_frame() {
            frames += 1;
            tracer.mark_frame(Some(&frame)).unwrap();
        }
    }
    tracer.finish().unwrap();
    assert!(frames >= 2, "expected at least two frames, got {frames}");

    // Read it back with gbtrace itself.
    let data = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let store = gbtrace::format::read::GbtraceStore::from_bytes(&data).unwrap();
    let header = store.header();
    assert_eq!(header.family, "nes");
    assert_eq!(header.family_def().id, "nes");
    assert_eq!(
        header.fields,
        [
            "pc", "a", "x", "y", "s", "p", "cycles", "control", "mask", "line", "dot", "counter"
        ]
    );
    // Self-describing metadata arrived without this crate doing anything.
    assert_eq!(header.field_defs.len(), header.fields.len());
    assert_eq!(header.instruction_addr_field.as_deref(), Some("pc"));
    assert!(!header.field_groups.is_empty());

    use gbtrace::store::TraceStore;
    assert!(store.entry_count() > 10_000);
    assert_eq!(store.frame_boundaries().len(), frames);

    // The program loops in $8000-$8008.
    let pc_col = store.field_col("pc").unwrap();
    for row in [1usize, 100, 10_000] {
        let pc = store.get_numeric(pc_col, row);
        assert!((0x8000..=0x8008).contains(&pc), "pc {pc:04x} at row {row}");
    }

    // The NES flag vocabulary works against the trace: INX wraps X past
    // 0xFF, setting Z.
    let hits = store
        .query_range("flag z becomes set", 0, store.entry_count())
        .unwrap();
    assert!(!hits.is_empty(), "no Z-flag transitions found");

    // Frame snapshots decode as indexed frames with the master palette.
    let payload = store.framebuffer(0).expect("frame payload");
    let frame = gbtrace::snapshot::IndexedFrame::from_bytes(&payload).expect("indexed frame");
    assert_eq!((frame.width, frame.height), (256, 240));
    assert_eq!(frame.palette.len(), 64);
    assert!(frame.pixels.iter().all(|&p| p < 64));
}
