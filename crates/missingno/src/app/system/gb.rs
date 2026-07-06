//! The Game Boy family's implementation of the system seam. One generic impl
//! serves both models; [`ConsoleUi`] carries the DMG↔CGB divergences.

use std::collections::BTreeSet;
use std::path::Path;

use missingno_gb::{
    BootRom, Console, GameBoy, cartridge::Cartridge, joypad::Button, ppu::model::PpuModel,
    serial_transfer::SerialLink,
};
use missingno_gbc::GameBoyColor;

use super::{FrameOutcome, SystemConsole, SystemDebugger};
use crate::app::console::ConsoleUi;
use crate::app::debugger::inspect::{ConsoleSnapshot, DebugView, InspectSource};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::FrameCapture;
use crate::app::screen::ScreenDisplay;

/// The registration point: picks the console model from the cartridge header —
/// CGB-aware ROMs get the CGB core, everything else the DMG core.
pub fn create_console(cartridge: Cartridge, boot_rom: Option<BootRom>) -> Box<dyn SystemConsole> {
    if cartridge.is_cgb() {
        Box::new(GameBoyColor::new(cartridge, boot_rom))
    } else {
        Box::new(GameBoy::new(cartridge, boot_rom))
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

    fn press_button(&mut self, button: Button) {
        Console::press_button(self, button);
    }

    fn release_button(&mut self, button: Button) {
        Console::release_button(self, button);
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

    fn cartridge(&self) -> &Cartridge {
        Console::cartridge(self)
    }

    fn set_link(&mut self, link: Box<dyn SerialLink>) {
        Console::set_link(self, link);
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        Box::new(GbDebugger {
            core: missingno_gb::debugger::Debugger::new(*self),
        })
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

    fn press_button(&mut self, button: Button) {
        self.core.game_boy_mut().press_button(button);
    }

    fn release_button(&mut self, button: Button) {
        self.core.game_boy_mut().release_button(button);
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

    fn inspect(&self) -> &dyn InspectSource {
        self.core.game_boy()
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        Box::new(ConsoleSnapshot::capture(self.core.game_boy(), frame))
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        let console = self.core.game_boy();
        RunningStatus {
            pc: console.cpu().ir_address,
            sp: console.cpu().stack_pointer,
            ly: console.ppu().video.ly(),
            mode: console.ppu().mode(),
            frame,
        }
    }

    fn cartridge(&self) -> &Cartridge {
        self.core.game_boy().cartridge()
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
