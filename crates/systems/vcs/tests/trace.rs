//! End-to-end morepork capture: run a minimal kernel, write a trace, and
//! read it back through morepork's own reader. The trace is authored from the
//! console's hardware state schema (the same vocabulary as the save-state
//! framing). The VCS has no hardware frame — the kernel's sync pattern decides
//! the height — so this also pins the per-frame-dimensions path.
#![cfg(feature = "morepork")]

use missingno_vcs::DumpFit;
use missingno_vcs::console::Vcs;
use missingno_vcs::trace::{TraceScope, Tracer, Trigger, step_instruction_counted};

/// A 4 KiB cartridge with a minimal 262-line NTSC kernel: 3 lines of
/// VSYNC, then 259 counted WSYNC lines of solid background colour.
fn test_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x1000];
    let program = [
        0xa9, 0x02, // lda #$02
        0x85, 0x00, // sta VSYNC
        0x85, 0x02, // sta WSYNC   } three VSYNC lines
        0x85, 0x02, // sta WSYNC
        0x85, 0x02, // sta WSYNC
        0xa9, 0x00, // lda #$00
        0x85, 0x00, // sta VSYNC
        0x85, 0x01, // sta VBLANK  (beam visible)
        0xa9, 0x46, // lda #$46    (a red)
        0x85, 0x09, // sta COLUBK
        0xa2, 0x00, // ldx #$00    (256 iterations)
        0x85, 0x02, // sta WSYNC   <- loop
        0xca, // dex
        0xd0, 0xfb, // bne loop
        0x85, 0x02, // sta WSYNC   } three more lines to reach 259
        0x85, 0x02, // sta WSYNC
        0x85, 0x02, // sta WSYNC
        0x4c, 0x00, 0xf0, // jmp $f000
    ];
    rom[..program.len()].copy_from_slice(&program);
    rom[0xFFC] = 0x00; // reset vector -> $F000
    rom[0xFFD] = 0xF0;
    rom
}

#[test]
fn captures_a_readable_trace_with_emergent_frames() {
    let rom = test_rom();
    let mut vcs = Vcs::new(&rom, missingno_vcs::TvStandard::Ntsc, None, DumpFit::Exact).unwrap();

    let path = std::env::temp_dir().join(format!(
        "missingno-vcs-trace-test-{}.morepork",
        std::process::id()
    ));
    let mut tracer = Tracer::create(
        &path,
        &rom,
        vcs.tv_standard(),
        Trigger::Instruction,
        TraceScope::Observable,
    )
    .unwrap();

    // A bit over two frames of the kernel (one frame = 262 lines × 76
    // CPU cycles).
    let mut total_cycles = 0u64;
    let mut last_cycles = 0u16;
    let mut frames = 0;
    let mut heights = Vec::new();
    while total_cycles < 45_000 {
        tracer.capture(&vcs, last_cycles).unwrap();
        last_cycles = step_instruction_counted(&mut vcs);
        total_cycles += last_cycles as u64;
        if let Some(frame) = vcs.take_frame() {
            frames += 1;
            heights.push(frame.lines.len());
            tracer.mark_frame(Some(&frame)).unwrap();
        }
    }
    tracer.finish().unwrap();
    assert!(frames >= 2, "expected at least two frames, got {frames}");
    // The kernel writes 259 non-VSYNC lines per frame; the emergent
    // heights must reflect the software, not a hardware constant.
    assert!(
        heights[1..].iter().all(|&h| h == 259),
        "expected 259-line frames, got {heights:?}"
    );

    // Read it back with morepork itself.
    let data = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let store = morepork::format::read::MoreporkStore::from_bytes(&data).unwrap();
    let header = store.header();
    assert_eq!(header.system, "vcs");
    assert_eq!(header.isa, "6502");
    assert_eq!(header.system_def().id, "vcs");
    assert_eq!(header.instruction_addr_field.as_deref(), Some("pc"));
    // The columns are the schema's Tier-1 observable fields plus the two
    // bridge-owned observations, authored from the schema — not a catalogue.
    for expected in ["pc", "a", "x", "y", "s", "p", "color_bk", "cycles", "line"] {
        assert!(
            header.fields.iter().any(|f| f == expected),
            "trace is missing column {expected}"
        );
    }

    use morepork::store::TraceStore;
    assert!(store.entry_count() > 1000);
    assert_eq!(store.frame_boundaries().len(), frames);

    // Frame snapshots decode as indexed frames with the emergent height
    // and carry the solid COLUBK background ($46 >> 1 = 0x23).
    let payload = store.frame_payload(1).expect("frame 1 has a payload");
    let frame = morepork::snapshot::IndexedFrame::from_bytes(&payload).unwrap();
    assert_eq!(frame.width, 160);
    assert_eq!(frame.height, 259);
    assert_eq!(frame.palette.len(), 128);
    assert!(
        frame.pixels.contains(&0x23),
        "expected COLUBK pixels in the frame"
    );

    // The WSYNC stall shows up as multi-cycle instructions: a store that
    // parks the CPU costs most of a 76-cycle scanline.
    let indices = store
        .query_range("cycles&40", 0, store.entry_count())
        .unwrap();
    assert!(
        !indices.is_empty(),
        "expected WSYNC-stalled instructions with large cycle counts"
    );

    // The 6507's flags resolve through the shared 6502 vocabulary.
    let zero_set = store.query_range("flag z set", 0, 1000).unwrap();
    assert!(!zero_set.is_empty());
}

#[test]
fn full_scope_adds_the_deep_die_state() {
    // The Full scope opts in the schema's Tier-2a fields — the object counters,
    // ring phases, and beam position a gate-level producer would also emit.
    let rom = test_rom();
    let mut vcs = Vcs::new(&rom, missingno_vcs::TvStandard::Ntsc, None, DumpFit::Exact).unwrap();
    let path = std::env::temp_dir().join(format!(
        "missingno-vcs-trace-full-{}.morepork",
        std::process::id()
    ));
    let mut tracer = Tracer::create(
        &path,
        &rom,
        vcs.tv_standard(),
        Trigger::Instruction,
        TraceScope::Full,
    )
    .unwrap();
    for _ in 0..200 {
        tracer.capture(&vcs, 0).unwrap();
        step_instruction_counted(&mut vcs);
    }
    tracer.finish().unwrap();

    let data = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).ok();
    use morepork::store::TraceStore;
    let store = morepork::format::read::MoreporkStore::from_bytes(&data).unwrap();
    let header = store.header();
    for deep in ["beam", "p0_position", "mot_ripple", "ch0_pulse"] {
        assert!(
            header.fields.iter().any(|f| f == deep),
            "full scope is missing deep field {deep}"
        );
    }
}
