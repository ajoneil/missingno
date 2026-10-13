//! Max-speed throughput benchmark: emulate N frames unpaced, report fps.
//! `--observe` sets an observer that keeps the call stack, and drains it each frame.
//!
//! ```sh
//! cargo run --profile profiling -p missingno-gb --example bench-dmg -- <rom.gb> [frames] [--observe]
//! ```

use missingno_gb::observe::{Filters, Observer};
use missingno_gb::{GameBoy, cartridge::Cartridge};

const WARMUP_FRAMES: u32 = 100;
const GB_FPS: f64 = 59.7275;

fn run_frames(gb: &mut GameBoy, frames: u32) {
    let mut seen = 0;
    while seen < frames {
        if gb.step().new_screen {
            seen += 1;
            std::hint::black_box(gb.drain_audio_samples());
            if let Some(o) = gb.observer_mut() {
                std::hint::black_box(o.take_events());
            }
        }
    }
}

fn main() {
    let observe = std::env::args().any(|a| a == "--observe");
    let mut args = std::env::args().skip(1).filter(|a| a != "--observe");
    let rom_path = args
        .next()
        .expect("usage: bench-dmg <rom> [frames] [--observe]");
    let frames: u32 = args
        .next()
        .map(|f| f.parse().expect("frames must be a number"))
        .unwrap_or(1000);

    let rom = std::fs::read(&rom_path).expect("failed to read ROM");
    let mut gb = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
    if observe {
        gb.set_observer(Some(Observer::new(Filters::default())));
    }

    run_frames(&mut gb, WARMUP_FRAMES);
    let start = std::time::Instant::now();
    run_frames(&mut gb, frames);
    let elapsed = start.elapsed().as_secs_f64();

    let fps = frames as f64 / elapsed;
    println!(
        "{frames} frames in {elapsed:.3}s = {fps:.1} fps ({:.2}x realtime)",
        fps / GB_FPS
    );
}
