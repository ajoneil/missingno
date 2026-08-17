//! The Game Boy family's binding to the machine seam: the core the hooks drive
//! — the console under its debugging backend, plus the link port and the
//! battery-save format the frontend owns — and the hooks themselves. One
//! generic impl serves both models; [`ConsoleUi`] carries the DMG↔CGB
//! divergences (screen framing, state schema, and the debugger capture).

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use missingno_core::HighPass;
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::machine::{
    BoundaryState, CoreRun, CoreStop, Machine, MachineConsole, StateIdentity, StopSet,
};
use missingno_core::ports::{
    ControlDescriptor, PeripheralDescriptor, PeripheralId, PlugError, PortDescriptor, PortId,
    Provider,
};
use missingno_core::state::{StateRecord, SystemStateSchema};
use missingno_core::state_file::StateFrame;
use missingno_core::symbols::{Symbol, SymbolTable};
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, ControlSite, DebugView, InspectSnapshot, RunningStatus,
    StateError,
};
use missingno_core::video::{DisplayTechnology, Frame, RawFrame};
use missingno_core::waveform::ChannelWave;

use crate::cartridge::Cartridge;
use crate::cpu::instructions::{calls_subroutine, instruction_length};
use crate::debugger::cdl::{self, CdlWindow, CodeDataLog};
use crate::debugger::inspection::{ColorSnapshot, GbSnapshot};
use crate::debugger::{Debugger, watchables};
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
/// flag, its screen framing, its state schema, and the per-vblank capture the
/// debugger renders from.
pub trait ConsoleUi: Model {
    /// DMG renders through a user-selectable monochrome palette; CGB is
    /// colour. Gates the play-mode Display panel's palette picker.
    const MONOCHROME_PALETTE: bool;

    /// Whether this console pages work RAM (CGB), which decides whether the
    /// `wram-bank` watch is one it can name.
    const BANKS_WORK_RAM: bool;

    /// The state a per-vblank capture carries: the model-shared view, plus
    /// whatever extra register state the model draws.
    type Inspect: InspectSnapshot + Clone + 'static;

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

    /// What the display shows before the first frame completes.
    fn blank_display() -> Frame;

    /// The current screen in its pre-resolution domain (DMG shade indices, CGB
    /// RGB555 words) — the values the accuracy references compare in.
    fn raw_frame(console: &Console<Self>) -> RawFrame;

    /// Copy the state the debugger renders from off the console, so the UI
    /// reads it while the core runs on the emulation thread.
    fn inspect(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> Self::Inspect;

    /// The model-shared part of a capture, which the seam reads for the answers
    /// both models give identically.
    fn shared(state: &Self::Inspect) -> &GbSnapshot;

    /// A capture stamped with the UI's frame counter, ready to publish.
    fn snapshot(state: &Self::Inspect, frame: u64) -> DebugView;

    /// The decoded graphics surfaces (tile atlases, maps, object table) for this
    /// console. One builder serves the live console (paused) and the per-vblank
    /// snapshot (running); each model composes its own view — DMG's frontend-
    /// shaded single bank, CGB's two-bank CRAM view.
    fn graphics_view(console: &Console<Self>) -> GraphicsView;
}

impl ConsoleUi for Dmg {
    const MONOCHROME_PALETTE: bool = true;
    const BANKS_WORK_RAM: bool = false;

    type Inspect = GbSnapshot;

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

    /// The panel before the first frame reads as it does with the LCD off.
    fn blank_display() -> Frame {
        Frame::Console(Box::new(GbFrame::GameBoy(GameBoyScreen::Off)))
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

    fn inspect(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> GbSnapshot {
        let colors = ColorSnapshot::Dmg {
            sgb: console.sgb().is_some(),
        };
        GbSnapshot::capture(console, colors, frame, symbols, cdl)
    }

    fn shared(state: &GbSnapshot) -> &GbSnapshot {
        state
    }

    fn snapshot(state: &GbSnapshot, frame: u64) -> DebugView {
        Box::new(state.at_frame(frame))
    }

    fn graphics_view(console: &Console<Self>) -> GraphicsView {
        crate::debugger::graphics::dmg_graphics_view(console.ppu(), console.vram())
    }
}

/// SHA-256 of the cartridge ROM, so a save state can refuse a ROM it was not
/// written for.
fn rom_fingerprint(cartridge: &Cartridge) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(cartridge.rom()).into()
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

/// The pad moulded into the console's own case.
pub const PAD: &[ControlDescriptor] = &[
    ControlDescriptor::button(ControlRole::Start, "Start"),
    ControlDescriptor::button(ControlRole::Select, "Select"),
    ControlDescriptor::button(ControlRole::Action(0), "A"),
    ControlDescriptor::button(ControlRole::Action(1), "B"),
    ControlDescriptor::button(ControlRole::Up, "Up"),
    ControlDescriptor::button(ControlRole::Down, "Down"),
    ControlDescriptor::button(ControlRole::Left, "Left"),
    ControlDescriptor::button(ControlRole::Right, "Right"),
];

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

/// What the seam drives: the console under its debugging backend, the link-port
/// peripheral it reports, and the battery-save format the frontend supplies.
pub struct GbCore<M: ConsoleUi> {
    debugger: Debugger<M>,
    battery_save: BatterySave,
    link: PeripheralId,
    /// The display a completed step produced, waiting for the seam to take it.
    pending: Option<Frame>,
    /// Battery-backed writes seen since the seam last took the flag.
    sram_dirty: bool,
}

impl<M: ConsoleUi> GbCore<M> {
    fn new(console: Console<M>, battery_save: BatterySave, link: PeripheralId) -> Self {
        GbCore {
            debugger: Debugger::new(console),
            battery_save,
            link,
            pending: None,
            sram_dirty: false,
        }
    }

    fn console(&self) -> &Console<M> {
        self.debugger.game_boy()
    }

    fn console_mut(&mut self) -> &mut Console<M> {
        self.debugger.game_boy_mut()
    }

    /// A step result mapped for display: the console may show something (LCD
    /// off, SGB freeze) even when no new frame completed.
    fn display(&self, screen: Option<M::Screen>) -> Option<Frame> {
        M::screen_display(self.console(), screen)
    }

    fn cdl_window(&self) -> CdlWindow {
        let console = self.console();
        let bank = console.cartridge().switchable_rom_bank();
        self.debugger
            .cdl()
            .window(console.cpu().ir_address, |address| {
                cdl::rom_offset(address, bank)
            })
    }

    /// Why a run that stopped short of its boundary stopped: a watch names
    /// itself, otherwise the pc has reached a breakpoint.
    fn stop_reason(&self, stops: &StopSet, watch_hit: Option<inspect::Watch>) -> CoreStop {
        match watch_hit {
            Some(watch) => CoreStop::WatchHit(watch),
            None if stops.pc.contains(&(self.console().cpu().ir_address as u32)) => {
                CoreStop::Breakpoint
            }
            None => CoreStop::BudgetExhausted,
        }
    }
}

/// A Game Boy adapted to the machine seam. One generic system serves both
/// models; [`ConsoleUi`] carries the divergences.
pub struct GbSystem<M>(PhantomData<M>);

/// A Game Boy under the seam's console wrapper.
pub type GbConsole<M> = MachineConsole<GbSystem<M>>;

/// Wrap a console for the seam, binding its save states to the loaded ROM.
pub fn create_console<M: ConsoleUi + 'static>(
    console: Console<M>,
    battery_save: BatterySave,
) -> GbConsole<M>
where
    Console<M>: Send,
{
    create_console_with_link(console, battery_save, LINK_DISCONNECTED)
}

/// Wrap a console whose link port already carries a host-built peripheral: the
/// object went in through [`Console::set_link`], and `link` names which kind it
/// was so the port reads back truthfully.
pub fn create_console_with_link<M: ConsoleUi + 'static>(
    console: Console<M>,
    battery_save: BatterySave,
    link: PeripheralId,
) -> GbConsole<M>
where
    Console<M>: Send,
{
    let fingerprint = rom_fingerprint(console.cartridge());
    let title = console.cartridge().title().to_string();
    MachineConsole::new(GbCore::new(console, battery_save, link), title).with_identity(
        StateIdentity {
            rom_fingerprint: fingerprint,
        },
    )
}

impl<M: ConsoleUi + 'static> Machine for GbSystem<M>
where
    Console<M>: Send,
{
    type Core = GbCore<M>;
    /// The family's frame is the composed display: what a completed step shows
    /// reads console state (the LCD's enable, an SGB mask) the display hook
    /// cannot see.
    type Frame = Frame;
    type InspectState = M::Inspect;

    const FRAME_INTERVAL: Duration = FRAME_INTERVAL;
    // The run hooks drive the engine to the console's own boundaries — a
    // completed frame, a call's return address — and never count instructions;
    // the seam asks for a floor all the same.
    const RUN_BUDGET: u32 = 200_000;

    fn pc(core: &GbCore<M>) -> u16 {
        core.console().cpu().ir_address
    }

    fn peek(core: &GbCore<M>, address: u16) -> u8 {
        core.console().peek(address)
    }

    fn peek_region(core: &GbCore<M>, address: u32) -> u8 {
        core.debugger.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn InstructionSet> {
        Some(&crate::isa::Sm83)
    }

    fn step_instruction(core: &mut GbCore<M>) {
        let screen = core.debugger.step();
        core.pending = core.display(screen);
    }

    fn take_frame(core: &mut GbCore<M>) -> Option<Frame> {
        core.pending.take()
    }

    /// One frame on the console's own budget: run until the PPU presents,
    /// bounded at two frames' worth of dots so an off LCD cannot stall it.
    fn step_frame(core: &mut GbCore<M>) -> Option<Frame> {
        let console = core.console_mut();
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
        console.sync_ppu();
        core.sram_dirty |= sram_dirty;
        let screen = core.console().screen().clone();
        Some(
            core.display(Some(screen))
                .expect("a screen given always displays"),
        )
    }

    fn power_cycle(core: &mut GbCore<M>) {
        core.debugger.reset();
    }

    fn apply_control(core: &mut GbCore<M>, control: ControlId, input: ControlInput) {
        let (Some(button), ControlInput::Digital(pressed)) = (button_for_control(control), input)
        else {
            return;
        };
        if pressed {
            core.console_mut().press_button(button);
        } else {
            core.console_mut().release_button(button);
        }
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(core: &GbCore<M>, port: PortId) -> Option<PeripheralId> {
        (port == LINK_PORT).then_some(core.link)
    }

    fn plug(core: &mut GbCore<M>, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        let GbCore { debugger, link, .. } = core;
        plug_link(debugger.game_boy_mut(), link, port, peripheral)
    }

    fn integrated_controls() -> &'static [ControlDescriptor] {
        PAD
    }

    fn drain_audio_samples(core: &mut GbCore<M>) -> Vec<(f32, f32)> {
        core.console_mut().drain_audio_samples()
    }

    fn audio_coupling() -> Option<HighPass> {
        Some(crate::board::audio_coupling())
    }

    fn video_out(_core: &GbCore<M>) -> DisplayTechnology {
        DisplayTechnology::Lcd {
            native: NATIVE_SIZE,
            panel: M::LCD_PANEL,
            pixel_aspect: 1.0,
        }
    }

    fn display_frame(frame: &Frame) -> Frame {
        frame.clone()
    }

    fn blank_display() -> Frame {
        M::blank_display()
    }

    fn frame_raw(core: &GbCore<M>) -> Option<RawFrame> {
        Some(M::raw_frame(core.console()))
    }

    fn uses_monochrome_palette() -> bool {
        M::MONOCHROME_PALETTE
    }

    fn supports_sgb(core: &GbCore<M>) -> bool {
        core.console().cartridge().supports_sgb()
    }

    fn take_sram_dirty(core: &mut GbCore<M>) -> bool {
        std::mem::take(&mut core.sram_dirty)
    }

    fn battery_save(core: &GbCore<M>) -> Option<Vec<u8>> {
        (core.battery_save)(core.console().cartridge())
    }

    fn state_schema() -> Option<&'static SystemStateSchema> {
        M::state_schema()
    }

    fn read_state(core: &GbCore<M>) -> Option<StateRecord> {
        M::read_state(core.console())
    }

    /// A save is only faithful at an instruction boundary, where the CPU carries
    /// no micro-sequencer residue — a fetch boundary, or halted waiting on an
    /// interrupt.
    fn capture_boundary(core: &GbCore<M>) -> Result<BoundaryState, StateError> {
        let console = core.console();
        if !console.cpu().is_fetch_phase() && !console.cpu().is_halted() {
            return Err(StateError::NotAtBoundary);
        }
        let record = M::read_state(console).ok_or(StateError::Unsupported)?;
        Ok(BoundaryState {
            record,
            memory: M::capture_memory(console),
            frame: Some(state_frame(&M::raw_frame(console))),
        })
    }

    fn restore_boundary(
        core: &mut GbCore<M>,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        M::restore_state(core.console_mut(), record, memory.to_vec(), frame)
    }

    /// A call runs to the address it returns to; every other instruction steps.
    fn step_over_target(core: &GbCore<M>) -> Option<u16> {
        let console = core.console();
        let address = console.cpu().ir_address;
        let opcode = console.peek(address);
        calls_subroutine(opcode).then(|| address.wrapping_add(instruction_length(opcode)))
    }

    fn tick_name() -> Option<&'static str> {
        Some("dot")
    }

    fn step_tick(core: &mut GbCore<M>) {
        core.debugger.step_tcycle();
    }

    /// Run until the PPU presents a frame, a breakpoint fires, or a watch hits.
    /// The engine evaluates the watches at the sub-instruction points they are
    /// defined on, so the seam's stops are handed to it whole.
    fn run_frame(core: &mut GbCore<M>, stops: &StopSet) -> CoreRun<Frame> {
        let run = core.debugger.step_frame(stops);
        let stopped_early = run.screen.is_none();
        let frame = core.display(run.screen);
        let stop = match stopped_early {
            true => core.stop_reason(stops, run.watch_hit),
            false => CoreStop::Completed,
        };
        CoreRun { stop, frame }
    }

    /// Run to the address the call returns to, carrying out the newest frame
    /// completed on the way.
    fn run_step_over(core: &mut GbCore<M>, stops: &StopSet, return_address: u16) -> CoreRun<Frame> {
        let run = core.debugger.run_to(return_address, stops);
        let frame = core.display(run.screen);
        let stop = match core.console().cpu().ir_address == return_address {
            true => CoreStop::Completed,
            false => core.stop_reason(stops, run.watch_hit),
        };
        CoreRun { stop, frame }
    }

    fn memory_regions(core: &GbCore<M>) -> Vec<inspect::MemoryRegion> {
        core.debugger.memory_regions()
    }

    fn present_address(core: &GbCore<M>, address: u32) -> inspect::AddressDisplay {
        core.debugger.present_address(address)
    }

    fn locate_bank_window(core: &GbCore<M>, bank: u16, window: u32) -> Option<u32> {
        core.debugger.locate_bank_window(bank, window)
    }

    fn watchables() -> &'static [inspect::Watchable] {
        watchables(M::BANKS_WORK_RAM)
    }

    fn symbols(core: &GbCore<M>) -> Arc<SymbolTable> {
        core.debugger.symbols().clone()
    }

    /// A label takes the bank paged into the window it lands in; the fixed
    /// windows are all bank 0.
    fn add_symbol(core: &mut GbCore<M>, address: u32, name: String) {
        let address = address as u16;
        let bank = match address {
            0x4000..=0x7fff => core
                .console()
                .cartridge()
                .switchable_rom_bank()
                .unwrap_or(0),
            _ => 0,
        };
        core.debugger.add_user_symbol(Symbol {
            bank,
            address,
            name,
        });
    }

    fn remove_symbol(core: &mut GbCore<M>, symbol: &Symbol) {
        core.debugger.remove_user_symbol(symbol);
    }

    fn cdl_window(core: &GbCore<M>) -> CdlWindow {
        core.cdl_window()
    }

    fn load_sidecars(core: &mut GbCore<M>, rom_path: &Path) {
        core.debugger.set_symbols(SymbolTable::for_rom(rom_path));
        let rom_len = core.console().cartridge().rom_len();
        core.debugger
            .set_cdl(CodeDataLog::load(&rom_path.with_extension("cdl"), rom_len));
    }

    fn save_sidecars(core: &GbCore<M>, rom_path: &Path) {
        core.debugger.cdl().save(&rom_path.with_extension("cdl"));
        core.debugger.save_symbols(&rom_path.with_extension("sym"));
    }

    fn capture_trace(core: &mut GbCore<M>, path: &Path) -> Option<Frame> {
        #[cfg(feature = "morepork")]
        {
            let screen = core.debugger.capture_frame(path).ok()?;
            core.display(Some(screen))
        }
        #[cfg(not(feature = "morepork"))]
        {
            let _ = (core, path);
            None
        }
    }

    fn inspect(core: &GbCore<M>, frame_count: u64) -> M::Inspect {
        M::inspect(
            core.console(),
            frame_count,
            core.debugger.symbols().clone(),
            core.cdl_window(),
        )
    }

    fn set_graphics_capture(core: &mut GbCore<M>, on: bool) {
        core.console_mut().set_graphics_capture(on);
    }

    fn graphics_view(core: &GbCore<M>) -> Option<GraphicsView> {
        let console = core.console();
        console
            .graphics_capture()
            .then(|| M::graphics_view(console))
    }

    fn set_wave_capture(core: &mut GbCore<M>, on: bool) {
        core.console_mut().set_wave_capture(on);
    }

    fn channel_waves(core: &GbCore<M>) -> Option<Vec<ChannelWave>> {
        core.console().channel_waves()
    }

    fn register_groups(state: &M::Inspect) -> Vec<inspect::RegisterGroup> {
        state.register_groups()
    }

    fn sidebar_sections(state: &M::Inspect) -> Vec<inspect::Section> {
        state.sidebar_sections()
    }

    fn snapshot(state: &M::Inspect, frame: u64) -> DebugView {
        M::snapshot(state, frame)
    }

    fn running_status(state: &M::Inspect, frame: u64) -> RunningStatus {
        M::shared(state).running_status(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::inspect::Watch;
    use missingno_core::system::{StepOutcome, SystemConsole, SystemDebugger};

    use crate::cartridge::Cartridge;

    /// NOP; JP 0150 → CALL 0160 { LD A,42; RET } → JR self.
    fn call_program() -> Console<Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        rom[0x150..0x153].copy_from_slice(&[0xcd, 0x60, 0x01]);
        rom[0x153..0x155].copy_from_slice(&[0x18, 0xfe]);
        rom[0x160..0x162].copy_from_slice(&[0x3e, 0x42]);
        rom[0x162] = 0xc9;
        Console::new(Cartridge::new(rom, None), None)
    }

    fn debugger() -> Box<dyn SystemDebugger> {
        Box::new(create_console(call_program(), |_| None)).into_debugger()
    }

    #[test]
    fn a_breakpoint_stops_the_frame_where_it_is_set() {
        let mut debugger = debugger();
        debugger.set_breakpoint(0x0150);
        assert!(matches!(
            debugger.run_frame(),
            StepOutcome::Breakpoint { .. }
        ));
        assert_eq!(debugger.pc(), 0x0150);
    }

    #[test]
    fn a_watch_reaches_the_engines_condition() {
        let mut debugger = debugger();
        debugger.add_watch(Watch::single("pc", None, Some(0x0150)));
        assert!(matches!(debugger.run_frame(), StepOutcome::WatchHit(_)));
        assert_eq!(debugger.pc(), 0x0150);
    }

    /// An MBC1 cart of eight banks whose program pages `bank` into the `$4000`
    /// window and jumps there, where a self-loop parks the pc at `$4000`.
    fn bank_jump(bank: u8) -> Console<Dmg> {
        let mut rom = vec![0u8; 8 * 0x4000];
        rom[0x147] = 0x01; // MBC1
        rom[0x148] = 0x04; // 128 KB
        rom[0x100..0x108].copy_from_slice(&[
            0x3e, bank, // LD A, bank
            0xea, 0x00, 0x20, // LD ($2000), A — select ROM bank
            0xc3, 0x00, 0x40, // JP $4000
        ]);
        for b in 1..8 {
            rom[b * 0x4000..b * 0x4000 + 2].copy_from_slice(&[0x18, 0xfe]); // JR -2
        }
        Console::new(Cartridge::new(rom, None), None)
    }

    #[test]
    fn a_banked_watch_gates_on_the_mapped_bank() {
        let compound = Watch {
            terms: vec![
                missingno_core::inspect::WatchTerm {
                    key: "pc".into(),
                    address: None,
                    value: Some(0x4000),
                },
                missingno_core::inspect::WatchTerm {
                    key: "rom-bank".into(),
                    address: None,
                    value: Some(3),
                },
            ],
        };

        let mut right: Box<dyn SystemDebugger> =
            Box::new(create_console(bank_jump(3), |_| None)).into_debugger();
        right.add_watch(compound.clone());
        assert!(matches!(right.run_frame(), StepOutcome::WatchHit(_)));
        assert_eq!(right.pc(), 0x4000);

        // Bank 2 mapped: the pc still reaches $4000, but the bank term rejects.
        let mut wrong: Box<dyn SystemDebugger> =
            Box::new(create_console(bank_jump(2), |_| None)).into_debugger();
        wrong.add_watch(compound);
        assert!(matches!(wrong.run_frame(), StepOutcome::Completed { .. }));
        assert!(wrong.last_watch_hit().is_none());
    }

    #[test]
    fn step_over_runs_a_call_out_and_steps_everything_else() {
        let mut over_call = debugger();
        over_call.step(); // NOP
        over_call.step(); // JP → at the CALL
        assert_eq!(over_call.pc(), 0x0150);
        over_call.step_over();
        assert_eq!(over_call.pc(), 0x0153);

        // A plain instruction has no return address, so step-over steps it.
        let mut over_nop = debugger();
        over_nop.step_over();
        assert_eq!(over_nop.pc(), 0x0101);
    }

    #[test]
    fn graphics_capture_gates_the_decoded_surfaces() {
        let mut debugger = debugger();
        assert!(debugger.graphics().is_none());
        debugger.set_graphics_capture(true);
        assert!(debugger.graphics().is_some());
    }

    #[test]
    fn video_out_states_the_models_panel() {
        let console = create_console(call_program(), |_| None);
        match console.video_out() {
            DisplayTechnology::Lcd { native, panel, .. } => {
                assert_eq!(native, NATIVE_SIZE);
                assert_eq!(panel, <Dmg as Model>::LCD_PANEL);
            }
            other => panic!("the Game Boy drives an LCD, got {other:?}"),
        }
    }
}
