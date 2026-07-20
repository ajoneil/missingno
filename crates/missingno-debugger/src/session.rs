//! The transport-agnostic debugger session: it owns the boxed
//! [`SystemDebugger`] and the run bookkeeping (frame counter, last stop
//! reason), and exposes the debugger surface as plain data. HTTP — or any
//! second transport — encodes what these methods return; no transport logic
//! lives here.

use std::collections::BTreeSet;
use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::disasm::{ReadMemory, Row, window_after};
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{
    MemoryRegion, RegisterGroup, Section, Watch, WatchParam, WatchTerm, Watchable,
};
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{ControlId, ControlInput, RunningStatus, StepOutcome, SystemDebugger};
use missingno_core::video::{DisplayTechnology, RawFrame, RgbaFrame};
use missingno_core::waveform::ChannelWave;

/// Why the last stepping call returned. The transport-carried form of
/// [`StepOutcome`], dropping the displayable frame.
#[derive(Clone, Debug)]
pub enum StopReason {
    Completed,
    Breakpoint,
    Watch(Watch),
    BudgetExhausted,
}

/// One line of a disassembly window: a decoded instruction, or a byte the
/// code/data log flagged as data.
#[derive(Clone, Debug)]
pub struct DisasmLine {
    pub address: u32,
    pub is_data: bool,
    pub bytes: Vec<u8>,
    /// The decoded mnemonic; empty for a data byte.
    pub text: String,
    pub length: u8,
}

/// A side-effect-free view of the console address space for the disassembler.
struct PeekMem<'a>(&'a dyn SystemDebugger);

impl ReadMemory for PeekMem<'_> {
    fn read(&self, address: u32) -> u8 {
        self.0.peek(address)
    }
}

pub struct Session {
    debugger: Box<dyn SystemDebugger>,
    frame: u64,
    last_stop: StopReason,
}

impl Session {
    pub fn new(debugger: Box<dyn SystemDebugger>) -> Self {
        Session {
            debugger,
            frame: 0,
            last_stop: StopReason::Completed,
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn last_stop(&self) -> &StopReason {
        &self.last_stop
    }

    pub fn pc(&self) -> u32 {
        self.debugger.pc()
    }

    pub fn game_title(&self) -> String {
        self.debugger.game_title()
    }

    pub fn video_out(&self) -> DisplayTechnology {
        self.debugger.video_out()
    }

    fn record(&mut self, outcome: StepOutcome) -> StopReason {
        let completed_frame = matches!(
            &outcome,
            StepOutcome::Completed { frame: Some(_) } | StepOutcome::Breakpoint { frame: Some(_) }
        );
        if completed_frame {
            self.frame += 1;
        }
        let reason = match outcome {
            StepOutcome::Completed { .. } => StopReason::Completed,
            StepOutcome::Breakpoint { .. } => StopReason::Breakpoint,
            StepOutcome::WatchHit(watch) => StopReason::Watch(watch),
            StepOutcome::BudgetExhausted => StopReason::BudgetExhausted,
        };
        self.last_stop = reason.clone();
        reason
    }

    pub fn step(&mut self) -> StopReason {
        let outcome = self.debugger.step();
        self.record(outcome)
    }

    pub fn step_over(&mut self) -> StopReason {
        let outcome = self.debugger.step_over();
        self.record(outcome)
    }

    pub fn step_frame(&mut self) -> StopReason {
        let outcome = self.debugger.step_frame();
        self.record(outcome)
    }

    /// The name of this core's sub-instruction step unit, or `None` when the
    /// core's finest step is a whole instruction.
    pub fn tick_name(&self) -> Option<&'static str> {
        self.debugger.tick_name()
    }

    /// Advance one sub-instruction tick. Steps nothing on a core without
    /// sub-instruction granularity; the run bookkeeping is untouched, matching
    /// the finest-grained stepping path.
    pub fn step_tick(&mut self) {
        self.debugger.step_tick();
    }

    /// The compact running-status summary (pc, sp, and the one-line video
    /// position) for the current machine state.
    pub fn running_status(&self) -> RunningStatus {
        self.debugger.running_status(self.frame)
    }

    pub fn reset(&mut self) {
        self.debugger.reset();
        self.frame = 0;
        self.last_stop = StopReason::Completed;
    }

    /// Write the current machine state to `path` as a save file. Errors when the
    /// system has no save-state backend or the file cannot be written.
    pub fn save_state(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self
            .debugger
            .save_state()
            .ok_or("this system has no save-state backend")?;
        std::fs::write(path, bytes).map_err(|error| format!("could not write {path:?}: {error}"))
    }

    /// Restore the machine state from a save file at `path`. Errors (never
    /// panics) on a missing file, a state for a different system or ROM, an
    /// unsupported version, or a corrupt file.
    pub fn load_state(&mut self, path: &std::path::Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|error| format!("could not read {path:?}: {error}"))?;
        self.debugger
            .load_state(&bytes)
            .map_err(|error| error.to_string())
    }

    /// Set a breakpoint at a bus address. Breakpoints are bus-space by
    /// contract; the seam keys them as a 16-bit bus address, so a synthetic
    /// bank-complete address (the disassembly's above-the-bus rows) is rejected
    /// rather than truncated into a phantom bus stop — banked stops are the
    /// watch system's job.
    pub fn set_breakpoint(&mut self, address: u32) -> Result<(), String> {
        if address > u16::MAX as u32 {
            return Err(format!(
                "breakpoint address {address:#x} is outside the bus space"
            ));
        }
        self.debugger.set_breakpoint(address);
        Ok(())
    }

    pub fn clear_breakpoint(&mut self, address: u32) {
        self.debugger.clear_breakpoint(address);
    }

    pub fn breakpoints(&self) -> BTreeSet<u32> {
        self.debugger.breakpoints()
    }

    pub fn register_groups(&self) -> Vec<RegisterGroup> {
        self.debugger.register_groups()
    }

    /// The structured machine-state sidebar this core exposes — the semantic
    /// description a transport renders as its "describe machine" view.
    pub fn sidebar_sections(&self) -> Vec<Section> {
        self.debugger.sidebar_sections()
    }

    /// Apply an input to a family-interpreted control (the GUI's control path,
    /// for a headless transport).
    pub fn set_control(&mut self, control: ControlId, input: ControlInput) {
        self.debugger.set_control(control, input);
    }

    pub fn memory_regions(&self) -> Vec<MemoryRegion> {
        self.debugger.memory_regions()
    }

    pub fn peek(&self, address: u32) -> u8 {
        self.debugger.peek(address)
    }

    pub fn memory(&self, address: u32, len: u32) -> Vec<u8> {
        (0..len)
            .map(|i| self.debugger.peek(address.wrapping_add(i)))
            .collect()
    }

    pub fn watchables(&self) -> &'static [Watchable] {
        self.debugger.watchables()
    }

    pub fn watches(&self) -> Vec<Watch> {
        self.debugger.watches()
    }

    /// Validate a set of watch terms against this core's watchables and add
    /// the conjoined watch; returns the watch that was added.
    pub fn add_watch(&mut self, terms: Vec<WatchTerm>) -> Result<Watch, String> {
        let watch = validate_watch(self.debugger.watchables(), terms)?;
        self.debugger.add_watch(watch.clone());
        Ok(watch)
    }

    pub fn remove_watch(&mut self, terms: Vec<WatchTerm>) -> Result<Watch, String> {
        let watch = validate_watch(self.debugger.watchables(), terms)?;
        self.debugger.remove_watch(&watch);
        Ok(watch)
    }

    pub fn symbols(&self) -> Arc<SymbolTable> {
        self.debugger.symbols()
    }

    /// Enable or disable the debugger's per-channel waveform capture.
    pub fn set_wave_capture(&mut self, on: bool) {
        self.debugger.set_wave_capture(on);
    }

    /// The current per-channel waveform windows, or `None` when the core
    /// captures none or capture is disabled.
    pub fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        self.debugger.channel_waves()
    }

    /// Enable or disable the debugger's per-vblank graphics-surface capture.
    pub fn set_graphics_capture(&mut self, on: bool) {
        self.debugger.set_graphics_capture(on);
    }

    /// The current decoded graphics surfaces, or `None` when the core has none
    /// or graphics capture is disabled.
    pub fn graphics(&self) -> Option<GraphicsView> {
        self.debugger.graphics()
    }

    /// A resolved RGBA frame of the current screen, as it stands (paused).
    pub fn frame_rgba(&self) -> RgbaFrame {
        self.debugger.screen_display().resolve_rgba()
    }

    /// The current frame in its pre-resolution domain (the values the accuracy
    /// references compare in), or `None` when the core has no such surface.
    pub fn frame_raw(&self) -> Option<RawFrame> {
        self.debugger.frame_raw()
    }

    /// The forward disassembly window from `at`, `count` lines. Errors when the
    /// core has no decode-for-display instruction set.
    pub fn disassembly(&self, at: u32, count: usize) -> Result<Vec<DisasmLine>, String> {
        let isa = self
            .debugger
            .instruction_set()
            .ok_or("this core has no disassembler")?;
        let memory = PeekMem(self.debugger.as_ref());
        let cdl: CdlWindow = self.debugger.cdl_window();
        let mask = isa.address_mask();
        let rows = window_after(at, count, isa, &memory, Some(&cdl));
        let lines = rows
            .into_iter()
            .map(|row| match row {
                Row::Instruction(address) => {
                    // Keep any synthetic high bits above the ISA space so the
                    // decode reads the bank-complete store the row addresses,
                    // not the bus the low bits alias onto.
                    let base = address & !mask;
                    let raw: Vec<u8> = (0..isa.max_len())
                        .map(|offset| {
                            memory.read(base | (address.wrapping_add(offset as u32) & mask))
                        })
                        .collect();
                    let decoded = isa.decode(address, &raw);
                    let length = decoded.length.max(1);
                    DisasmLine {
                        address,
                        is_data: false,
                        bytes: raw[..(length as usize).min(raw.len())].to_vec(),
                        text: decoded.mnemonic,
                        length,
                    }
                }
                Row::Data(address) => DisasmLine {
                    address,
                    is_data: true,
                    bytes: vec![memory.read(address)],
                    text: String::new(),
                    length: 1,
                },
            })
            .collect();
        Ok(lines)
    }
}

/// Validate watch terms against a watchable table: each term's key must name a
/// watchable, and its parameters must match that watchable's shape.
pub fn validate_watch(watchables: &[Watchable], terms: Vec<WatchTerm>) -> Result<Watch, String> {
    if terms.is_empty() {
        return Err("a watch needs at least one term".to_string());
    }
    for term in &terms {
        let watchable = watchables
            .iter()
            .find(|w| w.key == term.key)
            .ok_or_else(|| format!("unknown watch key: {}", term.key))?;
        match watchable.param {
            WatchParam::None => {
                if term.address.is_some() || term.value.is_some() {
                    return Err(format!("watch '{}' takes no parameters", term.key));
                }
            }
            WatchParam::Address => {
                if term.address.is_none() {
                    return Err(format!("watch '{}' requires an address", term.key));
                }
                if term.value.is_some() {
                    return Err(format!("watch '{}' takes only an address", term.key));
                }
            }
            WatchParam::Value { bits } => {
                let value = term
                    .value
                    .ok_or_else(|| format!("watch '{}' requires a value", term.key))?;
                if term.address.is_some() {
                    return Err(format!("watch '{}' takes only a value", term.key));
                }
                let max = if bits >= 32 {
                    u32::MAX
                } else {
                    (1u32 << bits) - 1
                };
                if value > max {
                    return Err(format!(
                        "watch '{}' value {value} exceeds its {bits}-bit range",
                        term.key
                    ));
                }
            }
            WatchParam::AddressValue => {
                if term.address.is_none() || term.value.is_none() {
                    return Err(format!(
                        "watch '{}' requires an address and a value",
                        term.key
                    ));
                }
            }
        }
    }
    Ok(Watch { terms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_core::inspect::WatchParam;

    fn table() -> [Watchable; 4] {
        [
            Watchable {
                key: "vblank",
                label: "VBlank",
                param: WatchParam::None,
            },
            Watchable {
                key: "bus-write",
                label: "Bus write",
                param: WatchParam::Address,
            },
            Watchable {
                key: "scanline",
                label: "Scanline",
                param: WatchParam::Value { bits: 8 },
            },
            Watchable {
                key: "poke",
                label: "Poke",
                param: WatchParam::AddressValue,
            },
        ]
    }

    fn term(key: &str, address: Option<u32>, value: Option<u32>) -> WatchTerm {
        WatchTerm {
            key: key.to_string(),
            address,
            value,
        }
    }

    #[test]
    fn empty_terms_rejected() {
        assert!(validate_watch(&table(), vec![]).is_err());
    }

    #[test]
    fn unknown_key_rejected() {
        assert!(validate_watch(&table(), vec![term("nope", None, None)]).is_err());
    }

    #[test]
    fn none_param_takes_no_arguments() {
        assert!(validate_watch(&table(), vec![term("vblank", None, None)]).is_ok());
        assert!(validate_watch(&table(), vec![term("vblank", Some(0x40), None)]).is_err());
    }

    #[test]
    fn address_param_requires_address_only() {
        assert!(validate_watch(&table(), vec![term("bus-write", Some(0xff40), None)]).is_ok());
        assert!(validate_watch(&table(), vec![term("bus-write", None, None)]).is_err());
        assert!(validate_watch(&table(), vec![term("bus-write", Some(1), Some(2))]).is_err());
    }

    #[test]
    fn value_param_enforces_bit_width() {
        assert!(validate_watch(&table(), vec![term("scanline", None, Some(0x90))]).is_ok());
        assert!(validate_watch(&table(), vec![term("scanline", None, Some(0x100))]).is_err());
        assert!(validate_watch(&table(), vec![term("scanline", None, None)]).is_err());
    }

    #[test]
    fn address_value_requires_both() {
        assert!(validate_watch(&table(), vec![term("poke", Some(0xc000), Some(7))]).is_ok());
        assert!(validate_watch(&table(), vec![term("poke", Some(0xc000), None)]).is_err());
        assert!(validate_watch(&table(), vec![term("poke", None, Some(7))]).is_err());
    }

    #[test]
    fn multi_term_conjunction_validates_each() {
        let watch = validate_watch(
            &table(),
            vec![
                term("vblank", None, None),
                term("scanline", None, Some(0x50)),
            ],
        )
        .unwrap();
        assert_eq!(watch.terms.len(), 2);
        // One bad term fails the whole conjunction.
        assert!(
            validate_watch(
                &table(),
                vec![term("vblank", None, None), term("scanline", None, None)],
            )
            .is_err()
        );
    }
}
