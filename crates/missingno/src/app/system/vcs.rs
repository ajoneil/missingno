//! The Atari 2600's implementation of the system seam. Emulator-only for
//! now: the family reports no debugger backend, so the shell falls back to
//! plain emulation.

use std::time::Duration;

use missingno_gb::joypad::{Button, DirectionalPad};
use missingno_gb::serial_transfer::SerialLink;
use missingno_vcs::cartridge::CartridgeError;
use missingno_vcs::console::{JoystickDirection, Vcs};
use missingno_vcs::tia::VISIBLE_CLOCKS;
use rgb::RGB8;

use super::{FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::library::activity::{DisplayMode, FrameCapture, RgbaCapture};
use crate::app::screen::{IndexedFrame, ScreenDisplay};

pub const PLATFORM_NAME: &str = "Atari 2600";
pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// Nominal NTSC frame: 262 lines × 228 clocks at the 3.579545 MHz colour
/// clock. Kernels vary line counts; the pacing loop uses the convention.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_684);

/// Frames are emergent from VSYNC; bound the search so a kernel that never
/// syncs cannot stall the emulation thread.
const FRAME_BUDGET_LINES: usize = 1000;

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide).
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => matches!(rom.len(), 0x800 | 0x1000),
        _ => false,
    }
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(VcsConsole {
        vcs: Vcs::new(rom)?,
        title,
        last_frame: blank_frame(),
    }))
}

struct VcsConsole {
    vcs: Vcs,
    title: String,
    last_frame: IndexedFrame,
}

fn blank_frame() -> IndexedFrame {
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height: 192,
        pixels: vec![0; VISIBLE_CLOCKS * 192].into(),
        palette: ntsc_palette(),
    }
}

impl SystemConsole for VcsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = self.vcs.step_frame(FRAME_BUDGET_LINES).map(|frame| {
            let height = frame.lines.len() as u32;
            let mut pixels = Vec::with_capacity(frame.lines.len() * VISIBLE_CLOCKS);
            for line in &frame.lines {
                // TIA colour bytes drop bit 0; the palette is 7-bit indexed.
                pixels.extend(line.iter().map(|&p| p >> 1));
            }
            self.last_frame = IndexedFrame {
                width: VISIBLE_CLOCKS as u32,
                height,
                pixels: pixels.into(),
                palette: ntsc_palette(),
            };
            ScreenDisplay::Indexed(self.last_frame.clone())
        });
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.vcs.power_cycle();
    }

    fn press_button(&mut self, button: Button) {
        self.apply_button(button, true);
    }

    fn release_button(&mut self, button: Button) {
        self.apply_button(button, false);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        Vec::new()
    }

    fn screen_display(&self) -> ScreenDisplay {
        ScreenDisplay::Indexed(self.last_frame.clone())
    }

    fn capture_frame(&self, _use_sgb_colors: bool, _palette_name: &str) -> FrameCapture {
        let frame = &self.last_frame;
        let mut data = Vec::with_capacity(frame.pixels.len() * 4);
        for &index in frame.pixels.iter() {
            let color = frame
                .palette
                .get(index as usize)
                .copied()
                .unwrap_or(RGB8::new(0, 0, 0));
            data.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
        FrameCapture {
            pixels: Vec::new(),
            sgb: None,
            display_mode: DisplayMode::Palette(String::new()),
            cgb_rgba: None,
            rgba: Some(RgbaCapture {
                width: frame.width,
                height: frame.height,
                data,
            }),
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }

    fn set_link(&mut self, _link: Box<dyn SerialLink>) {}

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Err(self)
    }
}

impl VcsConsole {
    /// The interim mapping onto the Game Boy button vocabulary: directions
    /// pass through, A fires, Start/Select work the console switches.
    fn apply_button(&mut self, button: Button, pressed: bool) {
        match button {
            Button::DirectionalPad(pad) => {
                let direction = match pad {
                    DirectionalPad::Up => JoystickDirection::Up,
                    DirectionalPad::Down => JoystickDirection::Down,
                    DirectionalPad::Left => JoystickDirection::Left,
                    DirectionalPad::Right => JoystickDirection::Right,
                };
                self.vcs.set_joystick(direction, pressed);
            }
            Button::A | Button::B => self.vcs.set_fire(pressed),
            Button::Start => self.vcs.set_console_reset(pressed),
            Button::Select => self.vcs.set_console_select(pressed),
        }
    }
}

/// The 128-colour NTSC TIA palette (colour byte bits 7-1: hue 4, luma 3),
/// approximated from hue-angle chroma — a display-side calibratable stage,
/// not a hardware claim.
fn ntsc_palette() -> &'static [RGB8] {
    use std::sync::OnceLock;
    static PALETTE: OnceLock<[RGB8; 128]> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let mut palette = [RGB8::new(0, 0, 0); 128];
        for (index, entry) in palette.iter_mut().enumerate() {
            let hue = (index >> 3) & 0x0F;
            let luma = (index & 0x07) as f32;
            let y = 0.12 + 0.85 * (luma / 7.0);
            let (i, q) = if hue == 0 {
                (0.0, 0.0)
            } else {
                // Hue 1 starts gold and the phase walks the colour wheel.
                let angle = (103.0 - 25.7 * (hue as f32 - 1.0)).to_radians();
                let saturation = 0.28 - 0.02 * (luma / 7.0);
                (saturation * angle.cos(), saturation * angle.sin())
            };
            let r = y + 0.956 * i + 0.619 * q;
            let g = y - 0.272 * i - 0.647 * q;
            let b = y - 1.106 * i + 1.703 * q;
            *entry = RGB8::new(channel(r), channel(g), channel(b));
        }
        palette
    })
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0) as u8
}
