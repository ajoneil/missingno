//! Max-speed throughput benchmark: emulate N frames unpaced, report fps.
//!
//! ```sh
//! cargo run --profile profiling -p missingno-gbc --example bench -- <rom.gbc> [frames]
//! ```

use missingno_gb::cartridge::Cartridge;
use missingno_gbc::GameBoyColor;

const WARMUP_FRAMES: u32 = 100;
const GB_FPS: f64 = 59.7275;

fn run_frames(gbc: &mut GameBoyColor, frames: u32) {
    let mut seen = 0;
    while seen < frames {
        if gbc.step().new_screen {
            seen += 1;
            std::hint::black_box(gbc.drain_audio_samples());
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = args.next().expect("usage: bench <rom> [frames]");
    let frames: u32 = args
        .next()
        .map(|f| f.parse().expect("frames must be a number"))
        .unwrap_or(1000);

    let rom = std::fs::read(&rom_path).expect("failed to read ROM");
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), None);

    run_frames(&mut gbc, WARMUP_FRAMES);
    let start = std::time::Instant::now();
    run_frames(&mut gbc, frames);
    let elapsed = start.elapsed().as_secs_f64();

    let fps = frames as f64 / elapsed;
    println!(
        "{frames} frames in {elapsed:.3}s = {fps:.1} fps ({:.2}x realtime)",
        fps / GB_FPS
    );
}
