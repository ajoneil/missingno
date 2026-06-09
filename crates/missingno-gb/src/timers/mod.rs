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
    /// Writes to TIMA during this cycle are ignored.
    pub reloading: bool,
    /// Models g151: CLK9-clocked DFF that delays timer overflow
    /// before it reaches the IF register (g154). When mcycle()
    /// detects overflow, it sets this to true instead of returning
    /// the interrupt immediately. On the next CLK9 tick (next dot),
    /// this is drained and the interrupt is returned.
    pub g151_pending: bool,
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
            g151_pending: false,
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
            g151_pending: false,
        }
    }

    fn selected_bit_set(&self) -> bool {
        self.control.enabled() && (self.internal_counter & self.control.selected_bit()) != 0
    }

    fn increment_tima(&mut self) {
        if self.counter == 0xFF {
            self.counter = 0;
            self.overflow_pending = true;
        } else {
            self.counter += 1;
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

        if was_set && !is_set {
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
                    self.increment_tima();
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
                    self.increment_tima();
                }
            }
        }
    }

    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::TimerSnapshot) -> Self {
        Self {
            internal_counter: snap.internal_counter,
            counter: snap.tima,
            modulo: snap.tma,
            control: Control(snap.tac),
            overflow_pending: snap.overflow_pending,
            reloading: snap.reloading,
            g151_pending: false,
        }
    }
}
