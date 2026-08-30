use std::collections::BTreeSet;
use std::path::Path;

use crate::{Console, Dmg, Model, cpu_bus::BusAccessKind};
use cdl::CodeDataLog;
use std::sync::Arc;
use symbols::SymbolTable;

mod address_space;
pub mod cdl;
pub mod graphics;
pub mod inspection;
mod watch;
use missingno_core::inspect;
use missingno_core::machine::StopSet;
use missingno_core::symbols;

pub use watch::watchables;
pub(crate) use watch::{ROM_BANK_KEY, SRAM_BANK_KEY, WRAM_BANK_KEY};
use watch::{WatchCondition, watch_from_condition, watch_to_condition};

pub struct Debugger<M: Model = Dmg> {
    game_boy: Console<M>,
    /// Labels from the ROM's `.sym` sidecar; shared so snapshots ride along.
    symbols: Arc<SymbolTable>,
    /// How each ROM byte has been used, filled in as the debugger runs.
    cdl: CodeDataLog,
}

/// The seam's stops in the form this engine evaluates them: breakpoints as bus
/// addresses, each watch translated into its condition. Compiled once per run,
/// so a per-instruction check costs no allocation.
struct RunStops {
    breakpoints: BTreeSet<u16>,
    watches: Vec<WatchCondition>,
}

impl RunStops {
    fn compile(stops: &StopSet) -> Self {
        Self {
            breakpoints: stops.pc.iter().map(|&address| address as u16).collect(),
            watches: stops
                .watches
                .iter()
                .filter_map(watch_to_condition)
                .collect(),
        }
    }
}

/// What a run stopped with: the newest screen completed on the way, and the
/// watch that stopped it.
pub struct RunOutcome<S> {
    pub screen: Option<S>,
    pub watch_hit: Option<inspect::Watch>,
}

impl<M: Model> Debugger<M> {
    pub fn new(game_boy: Console<M>) -> Self {
        let cdl = CodeDataLog::new(game_boy.cartridge().rom_len());
        Self {
            game_boy,
            symbols: Arc::new(SymbolTable::default()),
            cdl,
        }
    }

    pub fn cdl(&self) -> &CodeDataLog {
        &self.cdl
    }

    pub fn set_cdl(&mut self, cdl: CodeDataLog) {
        self.cdl = cdl;
    }

    pub fn set_symbols(&mut self, symbols: SymbolTable) {
        self.symbols = Arc::new(symbols);
    }

    pub fn symbols(&self) -> &Arc<SymbolTable> {
        &self.symbols
    }

    pub fn add_user_symbol(&mut self, symbol: symbols::Symbol) {
        Arc::make_mut(&mut self.symbols).add_user(symbol);
    }

    pub fn remove_user_symbol(&mut self, symbol: &symbols::Symbol) {
        Arc::make_mut(&mut self.symbols).remove_user(symbol);
    }

    pub fn save_symbols(&self, path: &Path) {
        self.symbols.save(path);
    }

    pub fn game_boy(&self) -> &Console<M> {
        &self.game_boy
    }

    pub fn game_boy_mut(&mut self) -> &mut Console<M> {
        &mut self.game_boy
    }

    pub fn step(&mut self) -> Option<M::Screen> {
        let result = self.step_logged();
        let screen = self.presented_screen(&result);
        self.game_boy.sync_audio();
        self.game_boy.sync_ppu();
        screen
    }

    fn presented_screen(&self, result: &crate::execute::StepResult) -> Option<M::Screen> {
        result.new_screen.then(|| self.game_boy.screen().clone())
    }

    /// One frame interval of T-cycles — the most a frame-run may spend before
    /// yielding without a screen, as when an off LCD presents nothing.
    fn frame_tcycle_budget(&self) -> u32 {
        crate::ppu::screen::DOTS_PER_FRAME * self.game_boy.cpu_steps_per_dot() as u32
    }

    /// Step one instruction while logging its code/data usage into the CDL.
    fn step_logged(&mut self) -> crate::execute::StepResult {
        let before = self.game_boy.cpu().ir_address;
        let bank_before = self.game_boy.cartridge().switchable_rom_bank();
        let opcode = self.game_boy.read(before);
        let length = crate::cpu::instructions::instruction_length(opcode);

        let result = self.game_boy.step_recorded();

        self.cdl.mark(
            cdl::rom_offset(before, bank_before),
            cdl::CODE | cdl::INSTRUCTION_START,
        );
        for offset in 1..length {
            self.cdl.mark(
                cdl::rom_offset(before.wrapping_add(offset), bank_before),
                cdl::CODE,
            );
        }
        let instruction_end = before.wrapping_add(length);
        for access in self.game_boy.bus_trace() {
            let is_read = matches!(access.kind, BusAccessKind::Read | BusAccessKind::DmaRead);
            let in_instruction = access.address >= before && access.address < instruction_end;
            if is_read && !in_instruction {
                self.cdl
                    .mark(cdl::rom_offset(access.address, bank_before), cdl::DATA);
            }
        }

        let after = self.game_boy.cpu().ir_address;
        if after != instruction_end {
            let bank_after = self.game_boy.cartridge().switchable_rom_bank();
            let bits = if crate::cpu::instructions::calls_subroutine(opcode) {
                cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT
            } else {
                cdl::JUMP_TARGET
            };
            self.cdl.mark(cdl::rom_offset(after, bank_after), bits);
        }
        result
    }

    pub fn step_tcycle(&mut self) -> Option<M::Screen> {
        let screen = self.step_tcycle_free();
        self.game_boy.sync_audio();
        self.game_boy.sync_ppu();
        screen
    }

    /// One T-cycle without the observation syncs, for the same reason.
    fn step_tcycle_free(&mut self) -> Option<M::Screen> {
        if self.game_boy.step_tcycle() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    /// Run frames until the program counter reaches `address` — a call's return
    /// address — or a breakpoint or watch stops it first, carrying out the
    /// newest screen completed on the way.
    pub fn run_to(&mut self, address: u16, stops: &StopSet) -> RunOutcome<M::Screen> {
        let mut stops = RunStops::compile(stops);
        stops.breakpoints.insert(address);
        let mut last_screen = None;
        loop {
            let outcome = self.run_frame(&stops);
            match outcome.screen {
                Some(screen) => last_screen = Some(screen),
                None => {
                    return RunOutcome {
                        screen: last_screen,
                        watch_hit: outcome.watch_hit,
                    };
                }
            }
        }
    }

    pub fn step_frame(&mut self, stops: &StopSet) -> RunOutcome<M::Screen> {
        self.run_frame(&RunStops::compile(stops))
    }

    fn run_frame(&mut self, stops: &RunStops) -> RunOutcome<M::Screen> {
        let (screen, hit) = if stops.watches.is_empty() {
            (self.step_frame_simple(stops), None)
        } else {
            self.step_frame_watched(stops)
        };
        self.game_boy.sync_audio();
        self.game_boy.sync_ppu();
        RunOutcome {
            screen,
            watch_hit: hit.as_ref().map(watch_from_condition),
        }
    }

    fn step_frame_simple(&mut self, stops: &RunStops) -> Option<M::Screen> {
        let mut budget = self.frame_tcycle_budget();
        loop {
            let result = self.step_logged();
            let screen = self.presented_screen(&result);
            if screen.is_some() || self.breakpoint_triggered(stops) {
                return screen;
            }
            budget = budget.saturating_sub(result.tcycles);
            if budget == 0 {
                return None;
            }
        }
    }

    fn step_frame_watched(
        &mut self,
        stops: &RunStops,
    ) -> (Option<M::Screen>, Option<WatchCondition>) {
        if stops.watches.iter().any(|w| w.needs_bus_trace()) {
            self.step_frame_watched_traced(stops)
        } else {
            self.step_frame_watched_dots(stops)
        }
    }

    fn step_frame_watched_traced(
        &mut self,
        stops: &RunStops,
    ) -> (Option<M::Screen>, Option<WatchCondition>) {
        let mut budget = self.frame_tcycle_budget();
        loop {
            let result = self.step_logged();
            let screen = self.presented_screen(&result);

            let hit = self.check_watchpoints(&stops.watches, self.game_boy.bus_trace());
            if hit.is_some() {
                return (screen, hit);
            }

            if screen.is_some() || self.breakpoint_triggered(stops) {
                return (screen, None);
            }

            budget = budget.saturating_sub(result.tcycles);
            if budget == 0 {
                return (None, None);
            }
        }
    }

    fn step_frame_watched_dots(
        &mut self,
        stops: &RunStops,
    ) -> (Option<M::Screen>, Option<WatchCondition>) {
        let mut budget = self.frame_tcycle_budget();
        loop {
            let screen = self.step_tcycle_free();

            if let Some(hit) = self.check_watchpoints(&stops.watches, &[]) {
                return (screen, Some(hit));
            }

            if screen.is_some() || self.breakpoint_triggered(stops) {
                return (screen, None);
            }

            budget -= 1;
            if budget == 0 {
                return (None, None);
            }
        }
    }

    fn breakpoint_triggered(&self, stops: &RunStops) -> bool {
        stops.breakpoints.contains(&self.game_boy.cpu().ir_address)
    }

    /// The SM83 register file as one inspection group.
    pub fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        inspection::cpu_register_groups(self.game_boy.cpu())
    }

    /// The address the debugger keys instructions on — the current opcode's
    /// fetch address, held for the whole instruction.
    pub fn pc(&self) -> u32 {
        self.game_boy.cpu().ir_address as u32
    }

    pub fn watchables(&self) -> &'static [inspect::Watchable] {
        watchables(self.game_boy.model().wram_image().is_some())
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
    }

    /// Capture a full T-cycle trace of one frame to a .morepork file. The full
    /// scope records the schema's deep pipeline state alongside the observable
    /// surface.
    #[cfg(feature = "morepork")]
    pub fn capture_frame(&mut self, path: impl AsRef<Path>) -> Result<M::Screen, String>
    where
        M: crate::system::ConsoleUi,
    {
        use crate::trace::{BootRom, TraceScope, Tracer, Trigger};

        let mut tracer = Tracer::create(
            path,
            &self.game_boy,
            Trigger::Tcycle,
            TraceScope::Full,
            BootRom::Skip,
            M::TRACE_MODEL_NAME,
        )
        .map_err(|e| format!("Failed to create tracer: {e}"))?;

        // Mark frame boundary at entry 0 so all entries belong to this frame.
        tracer
            .mark_frame()
            .map_err(|e| format!("Trace mark_frame error: {e}"))?;

        let mut trace_err = None;
        loop {
            let mut frame = false;
            let mut is_first = true;
            self.game_boy.execute_tcycle_observed(|gb, result| {
                if let Some(pixel) = result.pixel {
                    match pixel.pixel {
                        crate::ppu::TracePixel::Shade(shade) => tracer.push_pixel(shade),
                        crate::ppu::TracePixel::Rgb555(color) => tracer.push_pixel_rgb555(color),
                    }
                }
                frame |= result.new_screen;
                if is_first {
                    is_first = false;
                } else if let Err(e) = tracer.capture(gb) {
                    trace_err = Some(format!("Trace capture error: {e}"));
                } else {
                    tracer.advance_dot();
                }
            });
            if let Some(e) = trace_err {
                return Err(e);
            }
            if frame {
                break;
            }
        }

        tracer
            .finish()
            .map_err(|e| format!("Failed to finish trace: {e}"))?;

        Ok(self.game_boy.screen().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOP; JP 0150 → CALL 0160 { LD A,42; LD A,(0200); RET } → JR self.
    pub(super) fn traced_program_console() -> Console<Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        rom[0x150..0x153].copy_from_slice(&[0xcd, 0x60, 0x01]);
        rom[0x153..0x155].copy_from_slice(&[0x18, 0xfe]);
        rom[0x160..0x162].copy_from_slice(&[0x3e, 0x42]);
        rom[0x162..0x165].copy_from_slice(&[0xfa, 0x00, 0x02]);
        rom[0x165] = 0xc9;
        Console::new(
            crate::cartridge::Cartridge::new(rom, None, None).unwrap(),
            None,
        )
    }

    /// LD A,0; LDH (LCDC),A; JR self — turns the LCD off and spins.
    fn lcd_off_console() -> Console<Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x106].copy_from_slice(&[0x3e, 0x00, 0xe0, 0x40, 0x18, 0xfe]);
        Console::new(
            crate::cartridge::Cartridge::new(rom, None, None).unwrap(),
            None,
        )
    }

    #[test]
    fn an_off_lcd_frame_run_yields_after_one_frame_of_tcycles() {
        let mut debugger = Debugger::new(lcd_off_console());
        debugger.step_frame(&StopSet::default());
        let before = debugger.game_boy().chassis.clock.master_edge();
        let outcome = debugger.step_frame(&StopSet::default());
        assert!(outcome.screen.is_none());
        let tcycles = (debugger.game_boy().chassis.clock.master_edge() - before) / 2;
        let budget = u64::from(debugger.frame_tcycle_budget());
        assert!(
            (budget..budget + 64).contains(&tcycles),
            "one frame-run spent {tcycles} T-cycles"
        );
    }

    #[test]
    fn stepping_logs_code_data_and_control_flow() {
        let mut debugger = Debugger::new(traced_program_console());
        for _ in 0..6 {
            debugger.step();
        }

        let flags = |address| debugger.cdl().flags(cdl::rom_offset(address, Some(1)));
        assert_eq!(
            flags(0x0100) & (cdl::CODE | cdl::INSTRUCTION_START),
            cdl::CODE | cdl::INSTRUCTION_START
        );
        // JP operand byte: code, but not an instruction start.
        assert_eq!(flags(0x0103) & cdl::CODE, cdl::CODE);
        assert_eq!(flags(0x0103) & cdl::INSTRUCTION_START, 0);
        assert_eq!(
            flags(0x0150) & (cdl::CODE | cdl::JUMP_TARGET),
            cdl::CODE | cdl::JUMP_TARGET
        );
        assert_eq!(
            flags(0x0160) & (cdl::CODE | cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT),
            cdl::CODE | cdl::JUMP_TARGET | cdl::SUB_ENTRY_POINT
        );
        assert_eq!(flags(0x0200), cdl::DATA);
        assert_eq!(flags(0x0300), 0);
    }

    #[test]
    fn run_to_carries_a_call_to_its_return_address() {
        let mut debugger = Debugger::new(traced_program_console());
        debugger.step(); // NOP
        debugger.step(); // JP → at the CALL
        assert_eq!(debugger.game_boy().cpu().ir_address, 0x0150);
        debugger.run_to(0x0153, &StopSet::default());
        assert_eq!(debugger.game_boy().cpu().ir_address, 0x0153);
    }
}
