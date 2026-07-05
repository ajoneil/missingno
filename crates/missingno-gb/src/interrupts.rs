use bitflags::bitflags;

#[derive(Debug)]
pub enum Register {
    EnabledInterrupts,
    RequestedInterrupts,
}

#[derive(Clone, Copy)]
pub enum Interrupt {
    VideoBetweenFrames,
    VideoStatus,
    Timer,
    Serial,
    Joypad,
}

impl From<Interrupt> for InterruptFlags {
    fn from(interrupt: Interrupt) -> Self {
        match interrupt {
            Interrupt::VideoBetweenFrames => InterruptFlags::VIDEO_BETWEEN_FRAMES,
            Interrupt::VideoStatus => InterruptFlags::VIDEO_STATUS,
            Interrupt::Timer => InterruptFlags::TIMER,
            Interrupt::Serial => InterruptFlags::SERIAL,
            Interrupt::Joypad => InterruptFlags::JOYPAD,
        }
    }
}

bitflags! {
    #[derive(Copy, Clone)]
    pub struct InterruptFlags: u8 {
        const JOYPAD               = 0b00010000;
        const SERIAL               = 0b00001000;
        const TIMER                = 0b00000100;
        const VIDEO_STATUS         = 0b00000010;
        const VIDEO_BETWEEN_FRAMES = 0b00000001;

        const _OTHER = !0;
    }
}

impl Interrupt {
    pub fn vector(&self) -> u16 {
        match self {
            Interrupt::VideoBetweenFrames => 0x40,
            Interrupt::VideoStatus => 0x48,
            Interrupt::Timer => 0x50,
            Interrupt::Serial => 0x58,
            Interrupt::Joypad => 0x60,
        }
    }

    /// Highest-priority pending interrupt from an (IE ∧ IF) bit mask —
    /// the lowest set bit of bits 0-4, modelling the distributed-NOR
    /// priority chain. Bits 5-7 have no `irq_prio_bit` cell, so they
    /// never resolve to an interrupt.
    pub fn from_pending_bits(pending: u8) -> Option<Interrupt> {
        match pending.trailing_zeros() {
            0 => Some(Interrupt::VideoBetweenFrames),
            1 => Some(Interrupt::VideoStatus),
            2 => Some(Interrupt::Timer),
            3 => Some(Interrupt::Serial),
            4 => Some(Interrupt::Joypad),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Registers {
    pub enabled: InterruptFlags,
    pub requested: InterruptFlags,
    /// A PPU-fall IF set needs a half-edge to reach the CPU-visible FF0F byte;
    /// a double-speed read driving inside that window still sees the pre-set
    /// bit. The cell/dispatch/wake paths read `requested` directly.
    vblank_set_settle: u8,
    stat_set_settle: u8,
}

impl Registers {
    const IF_SET_SETTLE: u8 = 2;
    /// The five real IRQ bits (VBLANK, STAT, TIMER, SERIAL, JOYPAD).
    const IRQ_BITS: u8 = 0b0001_1111;

    pub fn new() -> Self {
        Self {
            enabled: InterruptFlags::empty(),
            requested: InterruptFlags::VIDEO_BETWEEN_FRAMES,
            vblank_set_settle: 0,
            stat_set_settle: 0,
        }
    }

    pub fn enabled(&self, interrupt: Interrupt) -> bool {
        self.enabled.contains(interrupt.into())
    }

    pub fn requested(&self, interrupt: Interrupt) -> bool {
        self.requested.contains(interrupt.into())
    }

    pub fn triggered(&self) -> Option<Interrupt> {
        let pending = self.enabled.bits() & self.requested.bits() & Self::IRQ_BITS;
        Interrupt::from_pending_bits(pending)
    }

    pub fn request(&mut self, interrupt: Interrupt) {
        self.requested.insert(interrupt.into());
    }

    /// A video IF set from the PPU fall path: arms the FF0F read-view settle
    /// on a 0→1 at double speed, then requests as normal.
    pub fn request_ppu_fall(&mut self, interrupt: Interrupt, double_speed: bool) {
        if double_speed && !self.requested(interrupt) {
            match interrupt {
                Interrupt::VideoBetweenFrames => self.vblank_set_settle = Self::IF_SET_SETTLE,
                Interrupt::VideoStatus => self.stat_set_settle = Self::IF_SET_SETTLE,
                _ => {}
            }
        }
        self.request(interrupt);
    }

    pub fn tick_set_settles(&mut self) {
        self.vblank_set_settle = self.vblank_set_settle.saturating_sub(1);
        self.stat_set_settle = self.stat_set_settle.saturating_sub(1);
    }

    /// The FF0F byte a CPU read sees: settling set bits held at pre-set.
    pub fn read_requested(&self) -> u8 {
        let mut bits = self.requested.bits();
        if self.vblank_set_settle > 0 {
            bits &= !InterruptFlags::VIDEO_BETWEEN_FRAMES.bits();
        }
        if self.stat_set_settle > 0 {
            bits &= !InterruptFlags::VIDEO_STATUS.bits();
        }
        bits | 0xE0
    }

    pub fn clear(&mut self, interrupt: Interrupt) {
        self.requested.remove(interrupt.into());
    }
}
