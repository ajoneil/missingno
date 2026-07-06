//! The Game Boy family's implementation of the system seam. One generic impl
//! serves both models; [`ConsoleUi`] carries the DMG↔CGB divergences.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_gb::{
    BootRom, Console, GameBoy,
    cartridge::Cartridge,
    debugger::{
        WatchCondition,
        cdl::{CdlWindow, CodeDataLog},
        symbols::{Symbol, SymbolTable},
    },
    joypad::Button,
    ppu::model::PpuModel,
    serial_transfer::SerialLink,
};
use missingno_gbc::GameBoyColor;

use super::{ControlId, ControlInput, FrameOutcome, SystemConsole, SystemDebugger};

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
use crate::app::library::activity::FrameCapture;
use crate::app::screen::ScreenDisplay;

/// How the Game Boy family's media appears in file dialogs, scanning, and
/// library metadata.
pub const PLATFORM_NAME: &str = "Nintendo Game Boy";
pub const ROM_FILTER_NAME: &str = "Game Boy ROM";
pub const ROM_EXTENSIONS: &[&str] = &["gb", "gbc"];
pub const DEFAULT_ROM_EXTENSION: &str = "gb";
pub const SAVE_FILTER_NAME: &str = "Game Boy Save";
pub const SAVE_EXTENSIONS: &[&str] = &["sav"];

/// One emulated frame at the DMG dot rate (~59.7 Hz); the CGB matches it
/// (double speed doubles CPU cycles per frame, not the frame rate).
const FRAME_INTERVAL: Duration = Duration::from_micros(16_740);

fn battery_save(cartridge: &Cartridge) -> Option<Vec<u8>> {
    if !cartridge.has_battery() {
        return None;
    }
    crate::sram::save_blob(cartridge, crate::sram::now_unix())
}

/// The registration point: picks the console model from the cartridge header —
/// CGB-aware ROMs get the CGB core, everything else the DMG core. The serial
/// link is a Game Boy peripheral, so it attaches here rather than at the seam.
pub fn create_console(
    cartridge: Cartridge,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
) -> Box<dyn SystemConsole> {
    if cartridge.is_cgb() {
        let mut console = GameBoyColor::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        Box::new(console)
    } else {
        let mut console = GameBoy::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        Box::new(console)
    }
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

    fn screen_display(&self) -> ScreenDisplay {
        M::screen_display(self, Some(self.screen().clone()))
            .expect("screen_display is always Some when given a screen")
    }

    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture {
        M::capture_frame(self, use_sgb_colors, palette_name)
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
    fn display(&self, screen: Option<M::Screen>) -> Option<ScreenDisplay> {
        M::screen_display(self.core.game_boy(), screen)
    }
}

impl<M: ConsoleUi + 'static> SystemDebugger for GbDebugger<M>
where
    Console<M>: Send,
    <M::Ppu as PpuModel>::Vram: Clone + Send + 'static,
{
    fn step(&mut self) -> Option<ScreenDisplay> {
        let screen = self.core.step();
        self.display(screen)
    }

    fn step_over(&mut self) -> Option<ScreenDisplay> {
        let screen = self.core.step_over();
        self.display(screen)
    }

    fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool) {
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

    fn symbols(&self) -> Arc<SymbolTable> {
        self.core.symbols().clone()
    }

    fn set_symbols(&mut self, symbols: SymbolTable) {
        self.core.set_symbols(symbols);
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

    fn save_symbols(&self, path: &Path) {
        self.core.save_symbols(path);
    }

    fn cdl_window(&self) -> CdlWindow {
        let console = self.core.game_boy();
        self.core.cdl().window(
            console.cpu().ir_address,
            console.cartridge().switchable_rom_bank(),
        )
    }

    fn load_cdl(&mut self, path: &Path) {
        let rom_len = self.core.game_boy().cartridge().rom_len();
        self.core.set_cdl(CodeDataLog::load(path, rom_len));
    }

    fn save_cdl(&self, path: &Path) {
        self.core.cdl().save(path);
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
            pc: console.cpu().ir_address,
            sp: console.cpu().stack_pointer,
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

    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture {
        M::capture_frame(self.core.game_boy(), use_sgb_colors, palette_name)
    }

    fn capture_trace(&mut self, path: &Path) -> Option<ScreenDisplay> {
        let screen = self.core.capture_frame(path).ok()?;
        self.display(Some(screen))
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(self.core.game_boy_take())
    }
}
