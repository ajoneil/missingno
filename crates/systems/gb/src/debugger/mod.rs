use std::collections::BTreeSet;
use std::path::Path;

use crate::{Console, Dmg, Model, cpu_bus::BusAccessKind, isa::Sm83};
use cdl::CodeDataLog;
use std::sync::Arc;
use symbols::SymbolTable;

mod address_space;
pub mod cdl;
pub mod graphics;
pub mod inspection;
mod watch;
use missingno_core::inspect;
use missingno_core::isa::InstructionSet;
use missingno_core::machine::StopSet;
use missingno_core::symbols;

pub use watch::{CpuRegister, WatchCondition, watchables};
pub(crate) use watch::{ROM_BANK_KEY, SRAM_BANK_KEY, WRAM_BANK_KEY};
use watch::{watch_from_condition, watch_to_condition};

pub struct Debugger<M: Model = Dmg> {
    game_boy: Console<M>,
    breakpoints: BTreeSet<u16>,
    watchpoints: Vec<WatchCondition>,
    last_watchpoint_hit: Option<WatchCondition>,
    /// Labels from the ROM's `.sym` sidecar; shared so snapshots ride along.
    symbols: Arc<SymbolTable>,
    /// How each ROM byte has been used, filled in as the debugger runs.
    cdl: CodeDataLog,
    /// T-cycle counter. Increments once per dot. Not hardware state —
    /// debugging/tracing infrastructure built on top of the emulation core.
    tcycle_count: u64,
}

impl<M: Model> Debugger<M> {
    pub fn new(game_boy: Console<M>) -> Self {
        let cdl = CodeDataLog::new(game_boy.cartridge().rom_len());
        Self {
            game_boy,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_watchpoint_hit: None,
            symbols: Arc::new(SymbolTable::default()),
            cdl,
            tcycle_count: 0,
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

    pub fn tcycle_count(&self) -> u64 {
        self.tcycle_count
    }

    pub fn step(&mut self) -> Option<M::Screen> {
        let screen = self.step_free();
        self.game_boy.sync_audio();
        self.game_boy.sync_ppu();
        screen
    }

    /// One instruction without the observation syncs — the run loops below own
    /// their own exit boundary.
    fn step_free(&mut self) -> Option<M::Screen> {
        let result = self.step_logged();
        self.tcycle_count += result.tcycles as u64;
        if result.new_screen {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
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
        self.tcycle_count += 1;
        if self.game_boy.step_tcycle() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    /// Run frames until the program counter reaches `address` — a call's return
    /// address — or a breakpoint or watch stops it first, carrying out the
    /// newest screen completed on the way.
    pub fn run_to(&mut self, address: u16) -> Option<M::Screen> {
        let temporary = self.breakpoints.insert(address);
        let mut last_screen = None;
        while let Some(screen) = self.step_frame() {
            last_screen = Some(screen);
        }
        if temporary {
            self.breakpoints.remove(&address);
        }
        last_screen
    }

    pub fn step_frame(&mut self) -> Option<M::Screen> {
        self.last_watchpoint_hit = None;
        let screen = if self.watchpoints.is_empty() {
            self.step_frame_simple()
        } else {
            self.step_frame_watched()
        };
        self.game_boy.sync_audio();
        self.game_boy.sync_ppu();
        screen
    }

    fn step_frame_simple(&mut self) -> Option<M::Screen> {
        loop {
            let screen = self.step_free();
            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched(&mut self) -> Option<M::Screen> {
        if self.watchpoints.iter().any(|w| w.needs_bus_trace()) {
            self.step_frame_watched_traced()
        } else {
            self.step_frame_watched_dots()
        }
    }

    fn step_frame_watched_traced(&mut self) -> Option<M::Screen> {
        loop {
            let result = self.step_logged();
            self.tcycle_count += result.tcycles as u64;
            let screen = if result.new_screen {
                Some(self.game_boy.screen().clone())
            } else {
                None
            };

            let hit = self
                .watchpoints
                .iter()
                .find(|condition| self.condition_matches(condition, self.game_boy.bus_trace()))
                .cloned();
            if let Some(hit) = hit {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched_dots(&mut self) -> Option<M::Screen> {
        loop {
            let screen = self.step_tcycle_free();

            if let Some(hit) = self.check_watchpoints(&[]) {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn breakpoint_triggered(&self) -> bool {
        self.breakpoints.contains(&self.game_boy.cpu().ir_address)
    }

    pub fn last_watchpoint_hit(&self) -> Option<&WatchCondition> {
        self.last_watchpoint_hit.as_ref()
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

    pub fn instruction_set(&self) -> &'static dyn InstructionSet {
        &Sm83
    }

    pub fn watchables(&self) -> &'static [inspect::Watchable] {
        watchables(self.game_boy.model().wram_image().is_some())
    }

    /// Take the seam's stop stores as this engine's own, translating each watch
    /// into the condition it evaluates. Called once per run, so a
    /// per-instruction check costs no allocation.
    pub fn load_stops(&mut self, stops: &StopSet) {
        self.breakpoints = stops.pc.iter().map(|&address| address as u16).collect();
        self.watchpoints = stops
            .watches
            .iter()
            .filter_map(watch_to_condition)
            .collect();
    }

    pub fn add_watch(&mut self, watch: inspect::Watch) {
        if let Some(condition) = watch_to_condition(&watch)
            && !self.watchpoints.contains(&condition)
        {
            self.watchpoints.push(condition);
        }
    }

    pub fn last_watch_hit(&self) -> Option<inspect::Watch> {
        self.last_watchpoint_hit.as_ref().map(watch_from_condition)
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
        self.tcycle_count = 0;
    }

    pub fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
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
            self.tcycle_count += 1;
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
        Console::new(crate::cartridge::Cartridge::new(rom, None), None)
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
        debugger.run_to(0x0153);
        assert_eq!(debugger.game_boy().cpu().ir_address, 0x0153);
        assert!(debugger.breakpoints().is_empty());
    }
}
