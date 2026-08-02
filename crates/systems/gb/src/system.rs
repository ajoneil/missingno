//! The Game Boy family's implementation of the system seam. One generic impl
//! serves both models; [`ConsoleUi`] carries the DMG↔CGB divergences (screen
//! framing and the debugger snapshot).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_core::HighPass;
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::ports::{
    ControlDescriptor, ControlKind, PeripheralDescriptor, PeripheralId, PlugError, PortDescriptor,
    PortId, Provider,
};
use missingno_core::state::{StateRecord, SystemStateSchema};
use missingno_core::state_file::{StateFrame, StateMeta, read_state_file, write_state_file};
use missingno_core::symbols::{Symbol, SymbolTable};
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, ControlSite, DebugView, FrameOutcome, RunningStatus,
    StateError, StepOutcome, SystemConsole, SystemDebugger,
};
use missingno_core::video::{DisplayTechnology, Frame, RawFrame};

use crate::cartridge::Cartridge;
use crate::debugger::cdl::{CdlWindow, CodeDataLog};
use crate::debugger::inspection::{ColorSnapshot, GbSnapshot};
use crate::debugger::{Debugger, WatchCondition};
use crate::frame::{GameBoyScreen, GbFrame, NATIVE_SIZE, SgbScreen};
use crate::joypad::Button;
use crate::sgb::MaskMode;
use crate::{Console, Dmg, Model};

/// One emulated frame at the DMG dot rate (~59.7 Hz); the CGB matches it
/// (double speed doubles CPU cycles per frame, not the frame rate).
const FRAME_INTERVAL: Duration = Duration::from_micros(16_740);

/// Serializes the cart's battery-backed contents for persistence. The frontend
/// owns the save-file format (the wall-clock RTC tail), so it supplies this.
pub type BatterySave = fn(&Cartridge) -> Option<Vec<u8>>;

/// How each console model presents to the system seam: its monochrome-palette
/// flag, its screen framing, and the per-vblank debugger snapshot it builds.
pub trait ConsoleUi: Model {
    /// DMG renders through a user-selectable monochrome palette; CGB is
    /// colour. Gates the play-mode Display panel's palette picker.
    const MONOCHROME_PALETTE: bool;

    /// This model's hardware state schema, if it authors one. DMG returns its
    /// schema; CGB composes it as the DMG fields plus its colour delta.
    fn state_schema() -> Option<&'static SystemStateSchema> {
        None
    }

    /// Read the console into a record keyed by the schema's field names — the
    /// save-state capture side. `None` when the model authors no schema.
    fn read_state(_console: &Console<Self>) -> Option<StateRecord> {
        None
    }

    /// The named RAM regions a save state carries for this model, keyed by
    /// schema span name.
    fn capture_memory(_console: &Console<Self>) -> Vec<(&'static str, Vec<u8>)> {
        Vec::new()
    }

    /// Restore the console in place from a validated record, its memory spans,
    /// and its saved framebuffer, at an instruction boundary. Errors (never
    /// panics) on a mid-instruction call or a record this model cannot restore.
    fn restore_state(
        _console: &mut Console<Self>,
        _record: &StateRecord,
        _memory: Vec<(String, Vec<u8>)>,
        _frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        Err(StateError::Unsupported)
    }

    /// The display for a step's screen result; `None` leaves the screen pane
    /// as-is.
    fn screen_display(console: &Console<Self>, new_screen: Option<Self::Screen>) -> Option<Frame>;

    /// The current screen in its pre-resolution domain (DMG shade indices, CGB
    /// RGB555 words) — the values the accuracy references compare in.
    fn raw_frame(console: &Console<Self>) -> RawFrame;

    /// A per-vblank inspection snapshot for the UI to render while running.
    fn snapshot(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> DebugView;

    /// The whole left-column sidebar for this console, composed from the shared
    /// section part-builders — each system decides its own sections and where
    /// its console-specific state sits.
    fn sidebar_sections(console: &Console<Self>) -> Vec<inspect::Section>;

    /// The decoded graphics surfaces (tile atlases, maps, object table) for this
    /// console. One builder serves the live console (paused) and the per-vblank
    /// snapshot (running); each model composes its own view — DMG's frontend-
    /// shaded single bank, CGB's two-bank CRAM view.
    fn graphics_view(console: &Console<Self>) -> GraphicsView;
}

impl ConsoleUi for Dmg {
    const MONOCHROME_PALETTE: bool = true;

    fn state_schema() -> Option<&'static SystemStateSchema> {
        Some(crate::state_schema::dmg_state_schema())
    }

    fn read_state(console: &Console<Self>) -> Option<StateRecord> {
        Some(crate::snapshot::read_shared_record(console))
    }

    fn capture_memory(console: &Console<Self>) -> Vec<(&'static str, Vec<u8>)> {
        crate::snapshot::capture_memory(console)
    }

    fn restore_state(
        console: &mut Console<Self>,
        record: &StateRecord,
        memory: Vec<(String, Vec<u8>)>,
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        console.restore_boundary(record, memory, frame)
    }

    fn screen_display(console: &Console<Self>, new_screen: Option<Self::Screen>) -> Option<Frame> {
        let video_enabled = console.ppu().control().video_enabled();
        if let Some(sgb) = console.sgb() {
            let render_data = sgb.render_data(video_enabled);
            if sgb.mask_mode == MaskMode::Freeze {
                Some(Frame::Console(Box::new(GbFrame::Sgb(SgbScreen::Freeze(
                    render_data,
                )))))
            } else {
                new_screen.map(|screen| {
                    Frame::Console(Box::new(GbFrame::Sgb(SgbScreen::Display(
                        screen,
                        render_data,
                    ))))
                })
            }
        } else if !video_enabled {
            Some(Frame::Console(Box::new(GbFrame::GameBoy(
                GameBoyScreen::Off,
            ))))
        } else {
            new_screen.map(|screen| {
                Frame::Console(Box::new(GbFrame::GameBoy(GameBoyScreen::Display(screen))))
            })
        }
    }

    fn raw_frame(console: &Console<Self>) -> RawFrame {
        use crate::ppu::screen::{NUM_SCANLINES, PIXELS_PER_LINE};
        let screen = console.screen();
        let pixels = (0..NUM_SCANLINES)
            .flat_map(|y| (0..PIXELS_PER_LINE).map(move |x| screen.pixel(x, y).0))
            .collect();
        RawFrame::Shade2 {
            width: NATIVE_SIZE.0,
            height: NATIVE_SIZE.1,
            pixels,
        }
    }

    fn snapshot(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> DebugView {
        let colors = ColorSnapshot::Dmg {
            sgb: console.sgb().is_some(),
        };
        let graphics = console
            .graphics_capture()
            .then(|| Self::graphics_view(console));
        Box::new(GbSnapshot::capture(
            console, colors, frame, symbols, cdl, graphics,
        ))
    }

    fn sidebar_sections(console: &Console<Self>) -> Vec<inspect::Section> {
        use crate::debugger::inspection::{AudioView, TimersView, dmg_sidebar_sections};
        dmg_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &TimersView::capture(console.timers()),
            &AudioView::capture(console.audio()),
            &console.cartridge().inspect(),
        )
    }

    fn graphics_view(console: &Console<Self>) -> GraphicsView {
        crate::debugger::graphics::dmg_graphics_view(console.ppu(), console.vram())
    }
}

/// A hex SHA-256 of the cartridge ROM, so a save state can refuse a ROM it was
/// not written for.
fn rom_sha256(cartridge: &Cartridge) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(cartridge.rom());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The current frame in its pre-resolution domain, as a save-state framebuffer
/// blob (informational — a restored console regenerates its display).
fn state_frame(raw: &RawFrame) -> StateFrame {
    use missingno_core::state::PixelFormat;
    match raw {
        RawFrame::Shade2 {
            width,
            height,
            pixels,
        } => StateFrame {
            width: *width,
            height: Some(*height),
            format: PixelFormat::Shade2,
            data: pixels.clone(),
        },
        RawFrame::Rgb555 {
            width,
            height,
            pixels,
        } => StateFrame {
            width: *width,
            height: Some(*height),
            format: PixelFormat::Rgb555,
            data: pixels.iter().flat_map(|word| word.to_le_bytes()).collect(),
        },
        RawFrame::Palette {
            width,
            height,
            pixels,
        } => StateFrame {
            width: *width,
            height: Some(*height),
            format: PixelFormat::Indexed8,
            data: pixels.clone(),
        },
    }
}

/// Serialize the console's boundary state into a save file: the schema-keyed
/// record, the RAM spans, and the current framebuffer. `None` when the model
/// authors no state schema, or the console is mid-instruction — a save is only
/// faithful at an instruction boundary, where the CPU carries no
/// micro-sequencer residue (a fetch boundary, or halted waiting on an
/// interrupt).
fn save_state_bytes<M: ConsoleUi>(console: &Console<M>) -> Option<Vec<u8>> {
    if !console.cpu().is_fetch_phase() && !console.cpu().is_halted() {
        return None;
    }
    let schema = M::state_schema()?;
    let record = M::read_state(console)?;
    let memory = M::capture_memory(console);
    let frame = state_frame(&M::raw_frame(console));
    let hash = rom_sha256(console.cartridge());
    let meta = StateMeta {
        system: schema.system,
        rom_sha256: Some(&hash),
        emulator: "missingno",
        emulator_version: env!("CARGO_PKG_VERSION"),
    };
    write_state_file(&meta, &record, &memory, Some(&frame)).ok()
}

/// Restore the console from a save file, rejecting a state for the wrong system
/// or ROM, an unsupported version, or a record that fails schema validation.
fn load_state_into<M: ConsoleUi>(console: &mut Console<M>, bytes: &[u8]) -> Result<(), StateError> {
    use missingno_core::state_file::StateFileError;

    let schema = M::state_schema().ok_or(StateError::Unsupported)?;
    let file = read_state_file(bytes).map_err(|error| match error {
        StateFileError::UnsupportedVersion(_) => StateError::VersionMismatch,
        _ => StateError::Corrupt,
    })?;
    if file.system != schema.system {
        return Err(StateError::WrongSystem);
    }
    if let Some(fingerprint) = &file.rom_sha256
        && *fingerprint != rom_sha256(console.cartridge())
    {
        return Err(StateError::IncompatibleRom);
    }
    let record = schema
        .record_from(file.fields)
        .map_err(|_| StateError::Corrupt)?;
    M::restore_state(console, &record, file.memory, file.frame.as_ref())
}

/// The pad moulded into the console's own case.
pub const PAD: &[ControlDescriptor] = &[
    button(ControlRole::Start, "Start"),
    button(ControlRole::Select, "Select"),
    button(ControlRole::Action(0), "A"),
    button(ControlRole::Action(1), "B"),
    button(ControlRole::Up, "Up"),
    button(ControlRole::Down, "Down"),
    button(ControlRole::Left, "Left"),
    button(ControlRole::Right, "Right"),
];

const fn button(role: ControlRole, label: &'static str) -> ControlDescriptor {
    ControlDescriptor {
        role,
        label,
        kind: ControlKind::Button,
    }
}

/// The serial socket on the console's left edge.
pub const LINK_PORT: PortId = PortId(0);
pub const LINK_DISCONNECTED: PeripheralId = PeripheralId(0);
pub const LINK_PRINTER: PeripheralId = PeripheralId(1);
pub const LINK_CABLE: PeripheralId = PeripheralId(2);

/// A printer needs its paper sink and a cable its far end, so both are built by
/// the host and arrive through [`Console::set_link`] rather than through a plug.
const LINK_PERIPHERALS: &[PeripheralDescriptor] = &[
    PeripheralDescriptor {
        id: LINK_DISCONNECTED,
        label: "Disconnected",
        provider: Provider::Console,
        controls: &[],
    },
    PeripheralDescriptor {
        id: LINK_PRINTER,
        label: "Game Boy Printer",
        provider: Provider::Host,
        controls: &[],
    },
    PeripheralDescriptor {
        id: LINK_CABLE,
        label: "Link cable",
        provider: Provider::Host,
        controls: &[],
    },
];

pub const PORTS: &[PortDescriptor] = &[PortDescriptor {
    port: LINK_PORT,
    label: "Link port",
    accepts: LINK_PERIPHERALS,
}];

fn plug_link<M: ConsoleUi>(
    console: &mut Console<M>,
    attached: &mut PeripheralId,
    port: PortId,
    peripheral: PeripheralId,
) -> Result<(), PlugError> {
    if port != LINK_PORT {
        return Err(PlugError::UnknownPort);
    }
    match LINK_PERIPHERALS.iter().find(|kind| kind.id == peripheral) {
        None => Err(PlugError::NotAccepted),
        Some(kind) if kind.provider == Provider::Host => Err(PlugError::HostProvided),
        Some(_) => {
            console.set_link(Box::new(crate::serial_transfer::Disconnected::new()));
            *attached = peripheral;
            Ok(())
        }
    }
}

/// Every Game Boy control is a button on the integrated pad; nothing else the
/// seam can name reaches the joypad matrix.
fn button_for_control(control: ControlId) -> Option<Button> {
    use crate::joypad::DirectionalPad as Dpad;
    if control.site != ControlSite::Integrated {
        return None;
    }
    Some(match control.role {
        ControlRole::Start => Button::Start,
        ControlRole::Select => Button::Select,
        ControlRole::Action(0) => Button::A,
        ControlRole::Action(1) => Button::B,
        ControlRole::Up => Button::DirectionalPad(Dpad::Up),
        ControlRole::Down => Button::DirectionalPad(Dpad::Down),
        ControlRole::Left => Button::DirectionalPad(Dpad::Left),
        ControlRole::Right => Button::DirectionalPad(Dpad::Right),
        _ => return None,
    })
}

/// A Game Boy core adapted to the seam. One generic wrapper serves both models;
/// [`ConsoleUi`] carries the divergences.
pub struct GbConsole<M: ConsoleUi> {
    console: Console<M>,
    battery_save: BatterySave,
    link: PeripheralId,
}

impl<M: ConsoleUi> GbConsole<M> {
    pub fn new(console: Console<M>, battery_save: BatterySave) -> Self {
        Self::with_link(console, battery_save, LINK_DISCONNECTED)
    }

    /// Wrap a console whose link port already carries a host-built peripheral:
    /// the object went in through [`Console::set_link`], and `link` names which
    /// kind it was so the port reads back truthfully.
    pub fn with_link(console: Console<M>, battery_save: BatterySave, link: PeripheralId) -> Self {
        Self {
            console,
            battery_save,
            link,
        }
    }
}

impl<M: ConsoleUi + 'static> SystemConsole for GbConsole<M>
where
    Console<M>: Send,
{
    fn step_frame(&mut self) -> FrameOutcome {
        let console = &mut self.console;
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
        console.sync_audio();
        FrameOutcome {
            display: Some(SystemConsole::screen_display(self)),
            sram_dirty,
        }
    }

    fn reset(&mut self) {
        Console::reset(&mut self.console);
    }

    fn uses_monochrome_palette(&self) -> bool {
        M::MONOCHROME_PALETTE
    }

    fn supports_sgb(&self) -> bool {
        Console::cartridge(&self.console).supports_sgb()
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        let (Some(button), ControlInput::Digital(pressed)) = (button_for_control(control), input)
        else {
            return;
        };
        if pressed {
            Console::press_button(&mut self.console, button);
        } else {
            Console::release_button(&mut self.console, button);
        }
    }

    fn integrated_controls(&self) -> &'static [ControlDescriptor] {
        PAD
    }

    fn ports(&self) -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        (port == LINK_PORT).then_some(self.link)
    }

    fn plug(&mut self, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        plug_link(&mut self.console, &mut self.link, port, peripheral)
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        Console::drain_audio_samples(&mut self.console)
    }

    fn audio_coupling(&self) -> Option<HighPass> {
        Some(crate::board::audio_coupling())
    }

    fn screen_display(&self) -> Frame {
        M::screen_display(&self.console, Some(self.console.screen().clone()))
            .expect("screen_display is always Some when given a screen")
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Lcd {
            native: crate::frame::NATIVE_SIZE,
            panel: M::LCD_PANEL,
            pixel_aspect: 1.0,
        }
    }

    fn game_title(&self) -> String {
        Console::cartridge(&self.console).title().to_string()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        (self.battery_save)(Console::cartridge(&self.console))
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        M::state_schema()
    }

    fn read_state(&self) -> Option<StateRecord> {
        M::read_state(&self.console)
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(&self.console)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        load_state_into(&mut self.console, bytes)
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        Box::new(GbDebugger {
            core: Debugger::new(self.console),
            battery_save: self.battery_save,
            link: self.link,
        })
    }
}

/// A Game Boy core under the debugger backend, adapting it to the seam.
pub struct GbDebugger<M: ConsoleUi> {
    core: Debugger<M>,
    battery_save: BatterySave,
    link: PeripheralId,
}

impl<M: ConsoleUi> GbDebugger<M> {
    /// A step result mapped for display: the system may show something (LCD
    /// off, SGB freeze) even when no new frame completed.
    fn display(&self, screen: Option<M::Screen>) -> Option<Frame> {
        M::screen_display(self.core.game_boy(), screen)
    }

    fn cdl_window(&self) -> CdlWindow {
        let console = self.core.game_boy();
        self.core.cdl().window(
            console.cpu().ir_address,
            console.cartridge().switchable_rom_bank(),
        )
    }

    /// The live console, for a family extension surface to read model-specific
    /// state the object-safe seam does not expose.
    pub fn console(&self) -> &Console<M> {
        self.core.game_boy()
    }

    /// Advance one dot (T-cycle) — the finest step the seam's instruction- and
    /// frame-granularity stepping cannot express.
    pub fn step_tcycle(&mut self) {
        self.core.step_tcycle();
    }

    pub fn watchpoints(&self) -> &[WatchCondition] {
        self.core.watchpoints()
    }

    pub fn add_watchpoint(&mut self, condition: WatchCondition) {
        self.core.add_watchpoint(condition);
    }

    pub fn remove_watchpoint(&mut self, condition: &WatchCondition) {
        self.core.remove_watchpoint(condition);
    }

    pub fn clear_watchpoints(&mut self) {
        self.core.clear_watchpoints();
    }

    pub fn last_watchpoint_hit(&self) -> Option<&WatchCondition> {
        self.core.last_watchpoint_hit()
    }
}

impl<M: ConsoleUi + 'static> SystemConsole for GbDebugger<M>
where
    Console<M>: Send,
{
    /// One frame under the debugger: the breakpoints and watches still stop it,
    /// and the host learns why only through [`SystemDebugger::run_frame`].
    fn step_frame(&mut self) -> FrameOutcome {
        FrameOutcome {
            display: SystemDebugger::run_frame(self).into_frame(),
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.core.reset();
    }

    fn uses_monochrome_palette(&self) -> bool {
        M::MONOCHROME_PALETTE
    }

    fn supports_sgb(&self) -> bool {
        self.core.game_boy().cartridge().supports_sgb()
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

    fn integrated_controls(&self) -> &'static [ControlDescriptor] {
        PAD
    }

    fn ports(&self) -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        (port == LINK_PORT).then_some(self.link)
    }

    fn plug(&mut self, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        plug_link(self.core.game_boy_mut(), &mut self.link, port, peripheral)
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.core.game_boy_mut().drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<HighPass> {
        Some(crate::board::audio_coupling())
    }

    fn screen_display(&self) -> Frame {
        self.display(Some(self.core.game_boy().screen().clone()))
            .expect("screen_display is always Some when given a screen")
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Lcd {
            native: crate::frame::NATIVE_SIZE,
            panel: M::LCD_PANEL,
            pixel_aspect: 1.0,
        }
    }

    fn game_title(&self) -> String {
        self.core.game_boy().cartridge().title().to_string()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        (self.battery_save)(self.core.game_boy().cartridge())
    }

    fn frame_interval(&self) -> Duration {
        FRAME_INTERVAL
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        M::state_schema()
    }

    fn read_state(&self) -> Option<StateRecord> {
        M::read_state(self.core.game_boy())
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(self.core.game_boy())
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        load_state_into(self.core.game_boy_mut(), bytes)
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        self
    }
}

impl<M: ConsoleUi + 'static> SystemDebugger for GbDebugger<M>
where
    Console<M>: Send,
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

    fn run_frame(&mut self) -> StepOutcome {
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

    fn tick_name(&self) -> Option<&'static str> {
        Some("dot")
    }

    fn step_tick(&mut self) {
        self.core.step_tcycle();
    }

    fn frame_raw(&self) -> Option<RawFrame> {
        Some(M::raw_frame(self.core.game_boy()))
    }

    fn set_wave_capture(&mut self, on: bool) {
        self.core.game_boy_mut().set_wave_capture(on);
    }

    fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.core.game_boy().channel_waves()
    }

    fn set_graphics_capture(&mut self, on: bool) {
        self.core.game_boy_mut().set_graphics_capture(on);
    }

    fn graphics(&self) -> Option<GraphicsView> {
        let console = self.core.game_boy();
        console
            .graphics_capture()
            .then(|| M::graphics_view(console))
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

    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        M::sidebar_sections(self.core.game_boy())
    }

    fn memory_regions(&self) -> Vec<inspect::MemoryRegion> {
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

    fn bank_for(&self, address: u32) -> Option<u16> {
        match address {
            0x4000..=0x7FFF => self.core.game_boy().cartridge().switchable_rom_bank(),
            _ => None,
        }
    }

    fn present_address(&self, address: u32) -> inspect::AddressDisplay {
        self.core.present_address(address)
    }

    fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        self.core.locate_bank_window(bank, window)
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
        GbDebugger::cdl_window(self)
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
        M::snapshot(
            self.core.game_boy(),
            frame,
            self.core.symbols().clone(),
            self.cdl_window(),
        )
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        let console = self.core.game_boy();
        RunningStatus {
            pc: console.cpu().ir_address.into(),
            sp: console.cpu().stack_pointer.into(),
            video_label: "PPU",
            video_summary: format!(
                "{} · ly {}",
                crate::debugger::inspection::mode_label(console.ppu().mode()),
                console.ppu().video.ly()
            ),
            frame,
        }
    }

    fn capture_trace(&mut self, path: &Path) -> Option<Frame> {
        #[cfg(feature = "morepork")]
        {
            let screen = self.core.capture_frame(path).ok()?;
            self.display(Some(screen))
        }
        #[cfg(not(feature = "morepork"))]
        {
            let _ = path;
            None
        }
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(GbConsole {
            console: self.core.game_boy_take(),
            battery_save: self.battery_save,
            link: self.link,
        })
    }
}
