//! TIA timing skeleton: horizontal beam counter, WSYNC/RDY, VSYNC/VBLANK,
//! and backdrop-colour pixel output. Object rendering arrives with M2 —
//! writes latch into the register file so the pipeline can grow onto it.

pub const CLOCKS_PER_LINE: u16 = 228;
pub const HBLANK_CLOCKS: u16 = 68;
pub const VISIBLE_CLOCKS: usize = 160;

mod registers {
    pub const VSYNC: u16 = 0x00;
    pub const VBLANK: u16 = 0x01;
    pub const WSYNC: u16 = 0x02;
    pub const COLUBK: u16 = 0x09;
}

/// One finished scanline: 160 TIA colour indices plus its blanking state.
#[derive(Clone)]
pub struct Scanline {
    pub pixels: [u8; VISIBLE_CLOCKS],
    pub vsync: bool,
    pub vblank: bool,
}

pub struct Tia {
    registers: [u8; 64],
    beam: u16,
    vsync: bool,
    vblank: bool,
    /// Low while a WSYNC strobe holds the CPU; released at line start.
    pub cpu_ready: bool,
    line: [u8; VISIBLE_CLOCKS],
    finished_line: Option<Scanline>,
}

impl Default for Tia {
    fn default() -> Self {
        Self::new()
    }
}

impl Tia {
    pub fn new() -> Self {
        Tia {
            registers: [0; 64],
            beam: 0,
            vsync: false,
            vblank: false,
            cpu_ready: true,
            line: [0; VISIBLE_CLOCKS],
            finished_line: None,
        }
    }

    /// Advance one colour clock; completed lines surface via `take_line`.
    pub fn step_clock(&mut self) {
        if self.beam >= HBLANK_CLOCKS {
            let x = (self.beam - HBLANK_CLOCKS) as usize;
            self.line[x] = if self.vblank {
                0
            } else {
                self.registers[registers::COLUBK as usize] & 0xFE
            };
        }
        self.beam += 1;
        if self.beam == CLOCKS_PER_LINE {
            self.beam = 0;
            self.cpu_ready = true;
            self.finished_line = Some(Scanline {
                pixels: self.line,
                vsync: self.vsync,
                vblank: self.vblank,
            });
        }
    }

    pub fn take_line(&mut self) -> Option<Scanline> {
        self.finished_line.take()
    }

    /// Current colour clock within the line (0..228) — inspection only.
    pub fn beam(&self) -> u16 {
        self.beam
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let register = address & 0x3F;
        self.registers[register as usize] = value;
        match register {
            registers::VSYNC => self.vsync = value & 0x02 != 0,
            registers::VBLANK => self.vblank = value & 0x02 != 0,
            registers::WSYNC => self.cpu_ready = false,
            _ => {}
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        // Collision latches and input ports arrive with M2; the driven
        // bits read back 0 until then.
        let _ = address & 0x0F;
        0
    }
}
