//! Capture a short `.morepork` trace through the schema-driven bridge, for the
//! `/compare-traces` smoke. Two runs from different initial register states
//! produce two traces that align on the same instruction stream but diverge in
//! the observable register columns.
//!
//! Usage: `cargo run -p missingno-gb --example trace-capture --features morepork \
//!         -- <out.morepork> [initial_a_hex]`

#[cfg(feature = "morepork")]
fn main() {
    use missingno_gb::trace::{BootRom, TraceScope, Tracer, Trigger};
    use missingno_gb::{GameBoy, cartridge::Cartridge};

    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .expect("usage: trace-capture <out.morepork> [initial_a_hex]");
    let initial_a = args
        .next()
        .map(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).expect("hex byte"))
        .unwrap_or(0);

    // A tight loop at the cartridge entry: INC A; INC B; JR -4. Control flow is
    // register-independent, so both runs walk the same instruction addresses.
    let mut rom = vec![0u8; 0x8000];
    rom[0x0100] = 0x3C; // INC A
    rom[0x0101] = 0x04; // INC B
    rom[0x0102] = 0x18; // JR -4
    rom[0x0103] = 0xFC;

    let mut gb = GameBoy::new(Cartridge::new(rom, None), None);
    gb.cpu_mut().a = initial_a;

    let mut tracer = Tracer::create(
        &out,
        &gb,
        Trigger::Instruction,
        TraceScope::Observable,
        BootRom::Skip,
        "DMG-B",
    )
    .expect("create tracer");

    for _ in 0..256 {
        gb.sync_audio();
        tracer.capture(&gb).expect("capture");
        gb.step();
    }
    tracer.finish().expect("finish");
    eprintln!("wrote {out} (initial a = {initial_a:#04x})");
}

#[cfg(not(feature = "morepork"))]
fn main() {
    eprintln!("build with --features morepork");
}
