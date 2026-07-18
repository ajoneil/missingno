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
    debugger::cdl::{CdlWindow, CodeDataLog},
    joypad::Button,
    ppu::model::PpuModel,
    serial_transfer::SerialLink,
};
use missingno_gbc::GameBoyColor;

use super::{
    ControlId, ControlInput, FrameOutcome, MediaLoad, StepOutcome, SystemConsole, SystemDebugger,
};

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
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;

use crate::app::console::ConsoleUi;
use crate::app::debugger::inspect::{ConsoleSnapshot, DebugView};
use crate::app::emu_thread::RunningStatus;
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
            Box::new(GbConsole(console))
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(GbConsole(console))
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

/// A Game Boy core adapted to the seam. A newtype so the app can implement the
/// core seam trait for it — the console itself belongs to another crate. One
/// generic wrapper serves both models; [`ConsoleUi`] carries the divergences.
pub struct GbConsole<M: ConsoleUi>(Console<M>);

impl<M: ConsoleUi + 'static> SystemConsole for GbConsole<M>
where
    Console<M>: Send,
    <M::Ppu as PpuModel>::Vram: Clone + Send + 'static,
{
    fn step_frame(&mut self) -> FrameOutcome {
        let console = &mut self.0;
        let max = 70224 * 2 * console.cpu_steps_per_dot() as u32;
        let mut tcycles = 0;
        let mut sram_dirty = false;
        loop {
            let result = console.step();
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
        Console::reset(&mut self.0);
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
            Console::press_button(&mut self.0, button);
        } else {
            Console::release_button(&mut self.0, button);
        }
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        Console::drain_audio_samples(&mut self.0)
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(missingno_gb::board::audio_coupling())
    }

    fn screen_display(&self) -> Frame {
        M::screen_display(&self.0, Some(self.0.screen().clone()))
            .expect("screen_display is always Some when given a screen")
    }

    fn video_out(&self) -> VideoOut {
        VideoOut::Lcd {
            native: missingno_gb::frame::NATIVE_SIZE,
        }
    }

    fn game_title(&self) -> String {
        Console::cartridge(&self.0).title().to_string()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        battery_save(Console::cartridge(&self.0))
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>> {
        Ok(Box::new(GbDebugger {
            core: missingno_gb::debugger::Debugger::new(self.0),
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
    fn step(&mut self) -> StepOutcome {
        let screen = self.core.step();
        StepOutcome::Completed {
            frame: self.display(screen),
        }
    }

    fn step_over(&mut self) -> StepOutcome {
        let screen = self.core.step_over();
        StepOutcome::Completed {
            frame: self.display(screen),
        }
    }

    fn step_frame(&mut self) -> StepOutcome {
        let screen = self.core.step_frame();
        // The core stops early (no completed frame) on a breakpoint or watch;
        // `last_watch_hit` names which, without changing the stop condition.
        let stopped_early = screen.is_none();
        let frame = self.display(screen);
        if stopped_early {
            match self.core.last_watch_hit() {
                Some(watch) => StepOutcome::WatchHit(watch),
                None => StepOutcome::Breakpoint { frame },
            }
        } else {
            StepOutcome::Completed { frame }
        }
    }

    fn screen_display(&self) -> Frame {
        self.display(Some(self.core.game_boy().screen().clone()))
            .expect("screen_display is always Some when given a screen")
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

    fn set_breakpoint(&mut self, address: u32) {
        self.core.set_breakpoint(address as u16);
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.core.clear_breakpoint(address as u16);
    }

    fn breakpoints(&self) -> BTreeSet<u32> {
        self.core.breakpoints().iter().map(|&a| a as u32).collect()
    }

    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        self.core.register_groups()
    }

    fn memory_regions(&self) -> &'static [inspect::MemoryRegion] {
        self.core.memory_regions()
    }

    fn peek(&self, address: u32) -> u8 {
        self.core.peek(address)
    }

    fn pc(&self) -> u32 {
        self.core.pc()
    }

    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        Some(self.core.instruction_set())
    }

    fn watchables(&self) -> &'static [inspect::Watchable] {
        self.core.watchables()
    }

    fn add_watch(&mut self, watch: inspect::Watch) {
        self.core.add_watch(watch);
    }

    fn remove_watch(&mut self, watch: &inspect::Watch) {
        self.core.remove_watch(watch.clone());
    }

    fn watches(&self) -> Vec<inspect::Watch> {
        self.core.watches()
    }

    fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.core.last_watch_hit()
    }

    fn family_state(&self) -> &dyn std::any::Any {
        self.core.game_boy()
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

    fn add_symbol(&mut self, address: u32, name: String) {
        let address = address as u16;
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

    fn capture_trace(&mut self, path: &Path) -> Option<Frame> {
        let screen = self.core.capture_frame(path).ok()?;
        self.display(Some(screen))
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(GbConsole(self.core.game_boy_take()))
    }
}
