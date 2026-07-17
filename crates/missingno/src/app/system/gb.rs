//! The Game Boy family's implementation of the system seam. One generic impl
//! serves both models; [`ConsoleUi`] carries the DMG↔CGB divergences.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_core::symbols::{Symbol, SymbolTable};
use missingno_core::video::VideoOut;
use missingno_gb::{
    BootRom, Console, GameBoy,
    cartridge::Cartridge,
    debugger::{
        WatchCondition,
        cdl::{CdlWindow, CodeDataLog},
    },
    joypad::Button,
    ppu::model::PpuModel,
    serial_transfer::SerialLink,
};
use missingno_gbc::GameBoyColor;

use super::{ControlId, ControlInput, FrameOutcome, MediaLoad, SystemConsole, SystemDebugger};

/// The inverse of the seam's numeric convention; ids 8+ are not GB controls.
fn button_for_control(control: ControlId) -> Option<Button> {
    use missingno_gb::joypad::DirectionalPad::*;
    Some(match control.0 {
        0 => Button::Start,
        1 => Button::Select,
        2 => Button::A,
        3 => Button::B,
        4 => Button::DirectionalPad(Up),
        5 => Button::DirectionalPad(Down),
        6 => Button::DirectionalPad(Left),
        7 => Button::DirectionalPad(Right),
        _ => return None,
    })
}
use crate::app::console::ConsoleUi;
use crate::app::debugger::inspect::{ConsoleSnapshot, DebugView, Inspection};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{CaptureOptions, FrameCapture};
use crate::app::screen::Frame;

/// Dual-mode media ships as `.gbc` files, so the Game Boy platform's dialog
/// filter must include that extension too.
pub const ROM_EXTENSIONS: &[&str] = &["gb", "gbc"];
pub const GBC_ROM_EXTENSIONS: &[&str] = &["gbc"];
pub const DEFAULT_ROM_EXTENSION: &str = "gb";
pub const SAVE_FILTER_NAME: &str = "Game Boy Save";
pub const SAVE_EXTENSIONS: &[&str] = &["sav"];

/// The family's names for the shared control ids, indexed by id; also the
/// bindings UI's primary labels.
pub const CONTROL_LABELS: [&str; 8] = ["Start", "Select", "A", "B", "Up", "Down", "Left", "Right"];

/// One emulated frame at the DMG dot rate (~59.7 Hz); the CGB matches it
/// (double speed doubles CPU cycles per frame, not the frame rate).
const FRAME_INTERVAL: Duration = Duration::from_micros(16_740);

fn battery_save(cartridge: &Cartridge) -> Option<Vec<u8>> {
    if !cartridge.has_battery() {
        return None;
    }
    crate::sram::save_blob(cartridge, crate::sram::now_unix())
}

fn has_family_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gb") || e.eq_ignore_ascii_case("gbc"))
}

/// Any Game Boy family media: big enough to carry a header, and either
/// wearing the boot logo (real media must, or the console refuses to boot)
/// or a family file extension (hand-built test ROMs may skip the logo).
fn is_family_rom(path: &Path, rom: &[u8]) -> bool {
    rom.len() >= 0x150 && (Cartridge::peek_valid_header(rom) || has_family_extension(path))
}

/// Game Boy platform media: everything the family claims except CGB-required
/// cartridges — dual-mode media belongs here, even though it boots enhanced.
pub fn is_gb_rom(path: &Path, rom: &[u8]) -> bool {
    is_family_rom(path, rom) && !Cartridge::peek_cgb_only(rom)
}

/// Game Boy Color platform media: cartridges the header marks CGB-required.
pub fn is_gbc_rom(path: &Path, rom: &[u8]) -> bool {
    is_family_rom(path, rom) && Cartridge::peek_cgb_only(rom)
}

pub fn title_from_rom(rom: &[u8]) -> Option<String> {
    let title = Cartridge::peek_title(rom);
    (!title.is_empty()).then_some(title)
}

/// A cartridge from ROM + saved battery contents: any RTC tail in the save
/// restores the clock and catches it up on the time since the save.
fn build_cartridge(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
    let (ram, rtc) = match save_data {
        Some(blob) => {
            let (ram, rtc) = crate::sram::split_blob(blob);
            (Some(ram), rtc)
        }
        None => (None, None),
    };
    let mut cartridge = Cartridge::new(rom, ram);
    if let Some((snapshot, saved_at)) = rtc {
        let elapsed = crate::sram::now_unix().saturating_sub(saved_at);
        cartridge.restore_rtc(snapshot, elapsed);
    }
    cartridge
}

/// Receives the console `launch` selects. Two concrete arms rather than one
/// generic method so a caller can require its own model traits on each.
pub trait GbLaunch {
    type Output;
    fn dmg(self, console: GameBoy) -> Self::Output;
    fn cgb(self, console: GameBoyColor) -> Self::Output;
}

/// The one DMG-vs-CGB selection point for every executable path (GUI load,
/// trace, headless): CGB-aware media — enhanced or required — boots the CGB
/// core, like a cartridge slotted into a real GBC; DMG-only media boots the
/// DMG core.
pub fn launch<L: GbLaunch>(
    rom: Vec<u8>,
    save_data: Option<Vec<u8>>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    launcher: L,
) -> L::Output {
    let cartridge = build_cartridge(rom, save_data);
    let boot_rom = matching_boot_rom(boot_rom, cartridge.is_cgb());
    if cartridge.is_cgb() {
        let mut console = GameBoyColor::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.cgb(console)
    } else {
        let mut console = GameBoy::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.dmg(console)
    }
}

fn matching_boot_rom(boot_rom: Option<BootRom>, cgb_core: bool) -> Option<BootRom> {
    match (&boot_rom, cgb_core) {
        (Some(BootRom::Dmg(_)), true) | (Some(BootRom::Cgb(_)), false) => {
            eprintln!("warning: boot ROM model does not match the selected core; ignoring it");
            None
        }
        _ => boot_rom,
    }
}

/// The factory both platform descriptors register: the header picks the
/// core. The serial link is a Game Boy peripheral, so it is taken here; a
/// virtual printer sits on the link port by default, staying inert unless a
/// game prints, with prints landing in the game's folder.
pub fn create_console(media: MediaLoad) -> Option<Box<dyn SystemConsole>> {
    struct Boxed;
    impl GbLaunch for Boxed {
        type Output = Box<dyn SystemConsole>;
        fn dmg(self, console: GameBoy) -> Self::Output {
            Box::new(console)
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(console)
        }
    }
    let link = media.serial_link.take().or_else(|| {
        media
            .print_sink
            .map(|sink| Box::new(crate::printer::GbPrinter::new(sink)) as Box<dyn SerialLink>)
    });
    Some(launch(
        media.rom.to_vec(),
        media.save_data,
        media.boot_rom,
        link,
        Boxed,
    ))
}

impl<M: ConsoleUi + 'static> SystemConsole for Console<M>
where
    Console<M>: Send,
    <M::Ppu as PpuModel>::Vram: Clone + Send + 'static,
{
    fn step_frame(&mut self) -> FrameOutcome {
        let max = 70224 * 2 * self.cpu_steps_per_dot() as u32;
        let mut tcycles = 0;
        let mut sram_dirty = false;
        loop {
            let result = self.step();
            tcycles += result.tcycles;
            sram_dirty |= result.sram_dirty;
            if result.new_screen || tcycles >= max {
                break;
            }
        }
        FrameOutcome {
            display: Some(SystemConsole::screen_display(self)),
            sram_dirty,
        }
    }

    fn reset(&mut self) {
        Console::reset(self);
    }

    fn uses_monochrome_palette(&self) -> bool {
        M::MONOCHROME_PALETTE
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        let (Some(button), ControlInput::Digital(pressed)) = (button_for_control(control), input)
        else {
            return;
        };
        if pressed {
            Console::press_button(self, button);
        } else {
            Console::release_button(self, button);
        }
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        Console::drain_audio_samples(self)
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(missingno_gb::board::audio_coupling())
    }

    fn screen_display(&self) -> Frame {
        M::screen_display(self, Some(self.screen().clone()))
            .expect("screen_display is always Some when given a screen")
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Lcd {
            native: missingno_gb::frame::NATIVE_SIZE,
        }
    }

    fn capture_frame(&self, options: &CaptureOptions) -> FrameCapture {
        M::capture_frame(self, options)
    }

    fn game_title(&self) -> String {
        Console::cartridge(self).title().to_string()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        battery_save(Console::cartridge(self))
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Ok(Box::new(GbDebugger {
            core: missingno_gb::debugger::Debugger::new(*self),
        }))
    }
}

/// A Game Boy core under the debugger backend, adapting it to the seam.
pub struct GbDebugger<M: ConsoleUi> {
    core: missingno_gb::debugger::Debugger<M>,
}

impl<M: ConsoleUi> GbDebugger<M> {
    /// A step result mapped for display: the system may show something (LCD
    /// off, SGB freeze) even when no new frame completed.
    fn display(&self, screen: Option<M::Screen>) -> Option<Frame> {
        M::screen_display(self.core.game_boy(), screen)
    }
}

impl<M: ConsoleUi + 'static> SystemDebugger for GbDebugger<M>
where
    Console<M>: Send,
    <M::Ppu as PpuModel>::Vram: Clone + Send + 'static,
{
    fn step(&mut self) -> Option<Frame> {
        let screen = self.core.step();
        self.display(screen)
    }

    fn step_over(&mut self) -> Option<Frame> {
        let screen = self.core.step_over();
        self.display(screen)
    }

    fn step_frame(&mut self) -> (Option<Frame>, bool) {
        let screen = self.core.step_frame();
        let breakpoint_hit = screen.is_none();
        (self.display(screen), breakpoint_hit)
    }

    fn reset(&mut self) {
        self.core.reset();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        let (Some(button), ControlInput::Digital(pressed)) = (button_for_control(control), input)
        else {
            return;
        };
        if pressed {
            self.core.game_boy_mut().press_button(button);
        } else {
            self.core.game_boy_mut().release_button(button);
        }
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.core.game_boy_mut().drain_audio_samples()
    }

    fn set_breakpoint(&mut self, address: u16) {
        self.core.set_breakpoint(address);
    }

    fn clear_breakpoint(&mut self, address: u16) {
        self.core.clear_breakpoint(address);
    }

    fn breakpoints(&self) -> &BTreeSet<u16> {
        self.core.breakpoints()
    }

    fn add_watchpoint(&mut self, condition: WatchCondition) {
        self.core.add_watchpoint(condition);
    }

    fn remove_watchpoint(&mut self, condition: &WatchCondition) {
        self.core.remove_watchpoint(condition);
    }

    fn watchpoints(&self) -> &[WatchCondition] {
        self.core.watchpoints()
    }

    fn last_watchpoint_hit(&self) -> Option<WatchCondition> {
        self.core.last_watchpoint_hit().cloned()
    }

    fn inspect(&self) -> &dyn Inspection {
        self.core.game_boy()
    }

    fn platform(&self) -> super::Platform {
        M::PLATFORM
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Lcd {
            native: missingno_gb::frame::NATIVE_SIZE,
        }
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(missingno_gb::board::audio_coupling())
    }

    fn symbols(&self) -> Arc<SymbolTable> {
        self.core.symbols().clone()
    }

    fn add_symbol(&mut self, address: u16, name: String) {
        let bank = match address {
            0x4000..=0x7fff => self
                .core
                .game_boy()
                .cartridge()
                .switchable_rom_bank()
                .unwrap_or(0),
            _ => 0,
        };
        self.core.add_user_symbol(Symbol {
            bank,
            address,
            name,
        });
    }

    fn remove_symbol(&mut self, symbol: &Symbol) {
        self.core.remove_user_symbol(symbol);
    }

    fn cdl_window(&self) -> CdlWindow {
        let console = self.core.game_boy();
        self.core.cdl().window(
            console.cpu().ir_address,
            console.cartridge().switchable_rom_bank(),
        )
    }

    fn load_sidecars(&mut self, rom_path: &Path) {
        self.core.set_symbols(SymbolTable::for_rom(rom_path));
        let rom_len = self.core.game_boy().cartridge().rom_len();
        self.core
            .set_cdl(CodeDataLog::load(&rom_path.with_extension("cdl"), rom_len));
    }

    fn save_sidecars(&self, rom_path: &Path) {
        self.core.cdl().save(&rom_path.with_extension("cdl"));
        self.core.save_symbols(&rom_path.with_extension("sym"));
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        Box::new(ConsoleSnapshot::capture(
            self.core.game_boy(),
            frame,
            self.core.symbols().clone(),
            self.cdl_window(),
        ))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        let console = self.core.game_boy();
        RunningStatus {
            pc: console.cpu().ir_address.into(),
            sp: console.cpu().stack_pointer.into(),
            video_label: "PPU",
            video_summary: format!(
                "{} · ly {}",
                crate::app::debugger::sidebar::mode_display(console.ppu().mode()).0,
                console.ppu().video.ly()
            ),
            frame,
        }
    }

    fn game_title(&self) -> String {
        self.core.game_boy().cartridge().title().to_string()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        battery_save(self.core.game_boy().cartridge())
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn capture_frame(&self, options: &CaptureOptions) -> FrameCapture {
        M::capture_frame(self.core.game_boy(), options)
    }

    fn capture_trace(&mut self, path: &Path) -> Option<Frame> {
        let screen = self.core.capture_frame(path).ok()?;
        self.display(Some(screen))
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(self.core.game_boy_take())
    }
}
