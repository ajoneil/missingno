use crate::interrupts::Interrupt;
use registers::Control;
pub use registers::Register;

pub mod registers;

#[derive(Clone)]
pub struct Timers {
    pub internal_counter: u16,
    pub counter: u8,
    pub modulo: u8,
    pub control: Control,
    pub overflow_pending: bool,
    /// Set when TIMA is in the reload cycle (TMA being loaded into TIMA).
    /// Writes to TIMA during this cycle are ignored, and so is a DIV or TAC write's count.
    pub reloading: bool,
    /// Set in the M-cycle after the reload. MEXU releases just past the boundary: after NYDU
    /// captured, so MUGY held it reset and MERY can't see a wrap, but before SOGU falls, so it counts.
    pub reload_releasing: bool,
    /// Models g151: CLK9-clocked DFF that delays timer overflow
    /// before it reaches the IF register (g154). When mcycle()
    /// detects overflow, it sets this to true instead of returning
    /// the interrupt immediately. On the next CLK9 tick (next dot),
    /// this is drained and the interrupt is returned.
    pub g151_pending: bool,
    /// Set when `mcycle()` incremented TIMA on the selected bit's natural 1→0
    /// fall this M-cycle, so a coinciding speed-switch DIV reset doesn't
    /// double-count the same edge.
    pub tima_fell_this_mcycle: bool,
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}

impl Timers {
    /// Post-boot state at the M-cycle boundary CLK9↑ that opens the
    /// PC=$0100 fetch. dmg-sim-aligned: reg_div16=0xEAF3 (FF04=0xAB).
    /// Real DMG (no harness gap) would read 0x6AF3 here. The two are
    /// observationally indistinguishable by construction: bit 15
    /// (UPOF) has no consumer outside reg_div16 — FF04 reads
    /// bits[13:6], TAC selects bits 1/3/5/7, no other path reads UPOF.
    pub fn post_boot() -> Self {
        Self {
            internal_counter: 0xEAF3,
            counter: 0,
            modulo: 0,
            control: Control(0xf8),
            overflow_pending: false,
            reloading: false,
            reload_releasing: false,
            g151_pending: false,
            tima_fell_this_mcycle: false,
        }
    }

    /// Post-boot state with a model-specific divider phase at handoff.
    pub fn post_boot_with_counter(internal_counter: u16) -> Self {
        Self {
            internal_counter,
            ..Self::post_boot()
        }
    }

    /// Power-on state at the SM83's first M-cycle. dmg-sim-aligned:
    /// 0x8001 reaches 0xEAF3 after the boot ROM's 5,860,082 M-cycles.
    /// UKUP=1 from the standard first-tick toggle (D=~Q on the boundary
    /// CLK9↑ that releases SM83); UPOF=1 reflects ~32,768 M-cycles of
    /// divider free-run between `reset_div_n` deassert and `sys_reset`
    /// deassert in dmg-sim's harness. Real DMG (simultaneous deassert)
    /// would initialise to 0x0001 and reach 0x6AF3 at PC=0x0100 —
    /// observationally indistinguishable by construction (UPOF has no
    /// consumer outside reg_div16).
    pub fn new() -> Self {
        Self {
            internal_counter: 0x8001,
            counter: 0,
            modulo: 0,
            control: Control(0xf8),
            overflow_pending: false,
            reloading: false,
            reload_releasing: false,
            g151_pending: false,
            tima_fell_this_mcycle: false,
        }
    }

    fn selected_bit_set(&self) -> bool {
        self.control.enabled() && (self.internal_counter & self.control.selected_bit()) != 0
    }

    fn increment_tima(&mut self) {
        if self.counter == 0xFF {
            self.counter = 0;
            // NYDU holds 0 after a reload, so this wrap raises no MOBA: no reload, no interrupt.
            if !self.reload_releasing {
                self.overflow_pending = true;
            }
        } else {
            self.counter += 1;
        }
    }

    /// A DIV or TAC write that drops the selected bit increments TIMA mid-M-cycle.
    fn increment_tima_on_write(&mut self) {
        // MEXU holds the TIMA cells loading TMA all M-cycle, so the toggle is lost.
        if !self.reloading {
            self.increment_tima();
        }
    }

    /// Advance by one M-cycle. On hardware, DIV00 is clocked by BOGA
    /// (one pulse per M-cycle). The entire 16-bit ripple counter
    /// advances once per M-cycle.
    ///
    /// Overflow sets `g151_pending` instead of returning the interrupt
    /// immediately. The caller must drain via `take_pending_interrupt()`
    /// on the next CLK9 rising edge.
    pub fn mcycle(&mut self) {
        self.reload_releasing = self.reloading;
        self.reloading = false;
        if self.overflow_pending {
            self.overflow_pending = false;
            self.reloading = true;
            self.counter = self.modulo;
            self.g151_pending = true;
        }

        let was_set = self.selected_bit_set();
        self.internal_counter = self.internal_counter.wrapping_add(1);
        let is_set = self.selected_bit_set();

        self.tima_fell_this_mcycle = was_set && !is_set;
        if self.tima_fell_this_mcycle {
            self.increment_tima();
        }
    }

    /// Drain the g151 DFF. Models the CLK9 rising edge latching g151's
    /// output, which then clocks g154 to set the timer IF bit.
    pub fn take_pending_interrupt(&mut self) -> Option<Interrupt> {
        if self.g151_pending {
            self.g151_pending = false;
            Some(Interrupt::Timer)
        } else {
            None
        }
    }

    pub fn internal_counter(&self) -> u16 {
        self.internal_counter
    }

    /// Divider reset driven by the KEY1 speed switch (not a CPU FF04 write).
    /// DIV reads 0 right after the switch and the post-reset ramp is unchanged.
    /// The immediate 1→0 increment for the 4KHz tap (bit 7, the highest
    /// TAC-selectable divider bit) is sampled one BOGA M-cycle late — the reset
    /// ripples up to bit 7 a cycle behind the lower taps — so its edge reads the
    /// divider one M-cycle before the reset. Faster taps see it at the reset.
    pub fn reset_for_speed_switch(&mut self) {
        let sel = self.control.selected_bit();
        let edge_counter = if sel == 1 << 7 {
            self.internal_counter.wrapping_sub(1)
        } else {
            self.internal_counter
        };
        let edge = self.control.enabled() && (edge_counter & sel) != 0;
        self.internal_counter = 0;
        // Don't double-count: if this M-cycle's natural 1→0 fall already
        // incremented TIMA, the reset's coinciding edge is the same one.
        if edge && !self.tima_fell_this_mcycle {
            self.increment_tima();
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Divider => (self.internal_counter >> 6) as u8,
            Register::Counter => self.counter,
            Register::Modulo => self.modulo,
            Register::Control => self.control.0 | 0xF8,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Divider => {
                let was_set = self.selected_bit_set();
                self.internal_counter = 0;
                if was_set {
                    self.increment_tima_on_write();
                }
            }
            Register::Counter => {
                if !self.reloading {
                    // Writing to TIMA during the overflow delay cancels the reload and interrupt
                    self.overflow_pending = false;
                    self.counter = value;
                }
                // Writing to TIMA during the reload cycle is ignored (TMA wins)
            }
            Register::Modulo => {
                self.modulo = value;
                // Writing to TMA during the reload cycle also updates TIMA
                if self.reloading {
                    self.counter = value;
                }
            }
            Register::Control => {
                let was_set = self.selected_bit_set();
                self.control = Control(value);
                let is_set = self.selected_bit_set();
                if was_set && !is_set {
                    self.increment_tima_on_write();
                }
            }
        }
    }

    pub fn from_snapshot(snap: &crate::snapshot::TimerSnapshot) -> Self {
        Self {
            internal_counter: snap.internal_counter,
            counter: snap.tima,
            modulo: snap.tma,
            control: Control(snap.tac),
            overflow_pending: snap.overflow_pending,
            reloading: snap.reloading,
            reload_releasing: false,
            g151_pending: false,
            tima_fell_this_mcycle: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TIMA $FF wraps on a TAC write that drops the tapped bit, then enters the reload M-cycle.
    fn wrapping_on_tac_write(tma: u8) -> Timers {
        let mut timers = Timers {
            internal_counter: 0x0020,
            counter: 0xFF,
            modulo: tma,
            control: Control(0x07),
            ..Timers::new()
        };
        timers.mcycle();
        timers.write_register(Register::Control, 0x05);
        assert_eq!(timers.counter, 0x00);
        timers.mcycle();
        assert!(timers.reloading);
        assert_eq!(timers.counter, tma);
        timers
    }

    #[test]
    fn div_write_in_reload_cycle_loses_its_count() {
        let mut timers = wrapping_on_tac_write(0x30);
        assert!(timers.selected_bit_set());
        timers.write_register(Register::Divider, 0);
        assert_eq!(timers.counter, 0x30);
        assert!(!timers.overflow_pending);
    }

    #[test]
    fn tac_write_in_reload_cycle_loses_its_count() {
        let mut timers = wrapping_on_tac_write(0x30);
        assert!(timers.selected_bit_set());
        timers.write_register(Register::Control, 0x01);
        assert_eq!(timers.counter, 0x30);
        assert!(!timers.overflow_pending);
    }

    #[test]
    fn write_in_reload_cycle_with_tma_ff_does_not_wrap_again() {
        let mut timers = wrapping_on_tac_write(0xFF);
        timers.take_pending_interrupt();
        timers.write_register(Register::Divider, 0);
        assert_eq!(timers.counter, 0xFF);
        timers.mcycle();
        assert!(!timers.reloading);
        assert!(timers.take_pending_interrupt().is_none());
        assert_eq!(timers.counter, 0xFF);
    }

    /// In the reload M-cycle, with the tapped bit 1 about to fall at the boundary that closes it.
    fn reloading_before_a_fall(tma: u8) -> Timers {
        Timers {
            internal_counter: 0x0003,
            counter: tma,
            modulo: tma,
            control: Control(0x05),
            reloading: true,
            ..Timers::new()
        }
    }

    #[test]
    fn count_at_boundary_closing_reload_cycle_is_kept() {
        let mut timers = reloading_before_a_fall(0x30);
        timers.mcycle();
        assert!(timers.reload_releasing);
        assert_eq!(timers.counter, 0x31);
    }

    #[test]
    fn wrap_at_boundary_closing_reload_cycle_does_not_reload() {
        let mut timers = reloading_before_a_fall(0xFF);
        timers.mcycle();
        assert_eq!(timers.counter, 0x00);
        assert!(!timers.overflow_pending);
        timers.mcycle();
        assert!(!timers.reloading);
        assert!(timers.take_pending_interrupt().is_none());
        assert_eq!(timers.counter, 0x00);
    }
}
