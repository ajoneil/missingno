#[cfg(feature = "gbtrace")]
use super::clock::Edge;
use super::{Console, Model, cpu::mcycle::TCycle, cpu_bus::BusAccess, ppu};

mod blackout;
mod commit;
mod dma;
mod edges;
mod video;

/// Result of executing one instruction.
pub struct StepResult {
    /// Whether a new video frame was produced during this instruction.
    pub new_screen: bool,
    /// Whether battery-backed SRAM was written during this instruction.
    pub sram_dirty: bool,
    /// Number of T-cycles consumed by this instruction.
    pub tcycles: u32,
}

/// Result of executing one half-phase (rise or fall).
pub struct PhaseResult {
    /// Whether a new video frame was produced.
    pub new_screen: bool,
    /// Pixel pushed to the LCD during this phase, if any.
    pub pixel: Option<ppu::PixelOutput>,
}

/// The facts a rising master-clock edge settles in its pre-PPU-rise work,
/// carried from `rise_cpu_pre` to `rise_cpu_post`: whether the edge opened a
/// new M-cycle, and the CPU T-cycle the PPU rise is keyed to.
#[derive(Clone, Copy)]
struct RiseEdge {
    mcycle_boundary: bool,
    tcycle: TCycle,
}

/// The facts a falling master-clock edge settles before its PPU fall, carried
/// from `fall_cpu_pre` to `fall_cpu_post`: the CPU T-cycle, whether the edge
/// opened a new M-cycle, an LY value sampled pre-edge for a coincident FF44
/// read latch, and the PPU mode the HDMA trigger reads before the fall mutates
/// it.
#[derive(Clone, Copy)]
struct FallEdge {
    tcycle: TCycle,
    mcycle_boundary: bool,
    ly_at_latch: Option<u8>,
    pre_fall_mode: ppu::Mode,
}

impl<M: Model> Console<M> {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (result, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.chassis.bus_trace.enable();
        }

        // If step_tcycle() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        let mut tcycles = 0u32;
        if !self.chassis.cpu.at_instruction_boundary() {
            let r = self.step_instruction();
            new_screen |= r.new_screen;
            tcycles += r.tcycles;
        }
        let r = self.step_instruction();
        new_screen |= r.new_screen;
        tcycles += r.tcycles;

        self.resolve_stop(tcycles);
        self.manage_dma_hold();

        let sram_dirty = self.chassis.external.cartridge.take_sram_dirty();
        (
            StepResult {
                new_screen,
                sram_dirty,
                tcycles,
            },
            self.chassis.bus_trace.take(),
        )
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        self.chassis.cpu.bus.data_latch = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.chassis.cpu.take_instruction_boundary();

        // Speed-switch blackout: the CPU clock is held while the dot clock
        // keeps running. Drive one CPU M-cycle of held master edges through the
        // same `execute_phase` loop (the gate is `Held`, the CPU frozen) and
        // return, draining the blackout across step()s until the count empties.
        if self.chassis.cpu.is_stopped() && self.model.speed_switch_in_progress() {
            return self.step_blackout_chunk();
        }

        const TCYCLE_BUDGET: u32 = 400;
        let mut tcycles_remaining = TCYCLE_BUDGET;
        let mut tcycles = 0u32;

        loop {
            assert!(
                tcycles_remaining > 0,
                "step() exceeded {TCYCLE_BUDGET} T-cycle budget — possible infinite loop in CPU"
            );
            tcycles_remaining -= 1;

            new_screen |= self.execute_tcycle();
            tcycles += 1;
            if self.chassis.cpu.at_instruction_boundary() {
                break;
            }
        }
        // Don't drain sram_dirty here — let the caller (step_traced) do it
        // so the flag accumulates across multiple step_instruction calls.
        let sram_dirty = self.chassis.external.cartridge.sram_dirty;
        StepResult {
            new_screen,
            sram_dirty,
            tcycles,
        }
    }

    /// Advance one CPU T-cycle — its two master edges, rise then fall — as one
    /// straight-line flow. The [`TcycleSchedule`] read up front names where the
    /// dot's rise and fall land: at ÷1 the rise carries the dot rise and the
    /// fall the dot fall; at ÷2 the T-cycle carries one dot edge, on its rise.
    /// The unit every step loop and the debugger advance by. Returns whether a
    /// new frame was produced across the pair. A T-cycle boundary is always
    /// rise-aligned, so the pair starts on the rise edge and ends back on a rise.
    pub fn execute_tcycle(&mut self) -> bool {
        let schedule = self.chassis.clock.tcycle_schedule();
        let rise = self.tcycle_rise(schedule);
        let fall = self.tcycle_fall(schedule);
        rise.new_screen || fall.new_screen
    }

    /// Advance one CPU T-cycle, observing the machine after each of its two
    /// master edges — the gbtrace capture hook. `after_phase` runs after the
    /// rise then after the fall with that edge's [`PhaseResult`], so a tracer
    /// keeps its exact between-edges sample points. A `Break` from the rise's
    /// observer leaves the fall for the next call: the double-speed per-edge
    /// capture defers the paired edge when the instruction retires on the rise,
    /// leaving the clock parked on the fall — so a resuming call runs only the
    /// fall (the pre-advance dot phase reproduces this T-cycle's schedule, since
    /// the ÷2 rise leaves the dot phase untouched).
    #[cfg(feature = "gbtrace")]
    pub fn execute_tcycle_observed(
        &mut self,
        mut after_phase: impl FnMut(&mut Self, &PhaseResult) -> std::ops::ControlFlow<()>,
    ) -> bool {
        let schedule = self.chassis.clock.tcycle_schedule();
        let mut new_screen = false;
        if self.chassis.clock.cpu_edge() == Edge::Rise {
            let rise = self.tcycle_rise(schedule);
            new_screen |= rise.new_screen;
            if after_phase(self, &rise).is_break() {
                return new_screen;
            }
        }
        let fall = self.tcycle_fall(schedule);
        new_screen |= fall.new_screen;
        // The fall is the pair's last edge; nothing to defer, so its control
        // signal is irrelevant.
        let _ = after_phase(self, &fall);
        new_screen
    }

    /// Advance to the next T-cycle boundary. Returns true if a new frame was
    /// produced. Consumes the boundary flag so a following `step`/`step_traced`
    /// sees mid-instruction state.
    pub fn step_tcycle(&mut self) -> bool {
        let new_screen = self.execute_tcycle();
        self.chassis.cpu.take_instruction_boundary();
        new_screen
    }
}
