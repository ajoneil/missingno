//! The 315-5124 VDP: timing, ports, and interrupts. Rendering is
//! line-granular — each active line is drawn as it completes, from the
//! register state at that moment. Dot-level write capture within a line
//! is later accuracy work.

pub const DOTS_PER_LINE: u16 = 342;
pub const LINES_PER_FRAME: u16 = 262;
pub const ACTIVE_LINES: u16 = 192;
pub const PIXELS_PER_LINE: usize = 256;

mod status {
    pub const FRAME_INTERRUPT: u8 = 0x80;
}

/// One finished frame: CRAM indices per pixel plus the palette they
/// resolve through, snapshotted as the frame completed.
pub struct Frame {
    pub pixels: Vec<u8>,
    pub cram: [u8; 32],
}

pub struct Vdp {
    pub registers: [u8; 11],
    pub vram: Box<[u8; 0x4000]>,
    pub cram: [u8; 32],

    dot: u16,
    line: u16,

    address: u16,
    code: u8,
    control_latch: Option<u8>,
    read_buffer: u8,
    status: u8,
    line_interrupt_pending: bool,
    line_counter: u8,

    frame_pixels: Vec<u8>,
    finished_frame: Option<Frame>,
}

impl Default for Vdp {
    fn default() -> Self {
        Self::new()
    }
}

impl Vdp {
    pub fn new() -> Self {
        Vdp {
            registers: [0; 11],
            vram: Box::new([0; 0x4000]),
            cram: [0; 32],
            dot: 0,
            line: 0,
            address: 0,
            code: 0,
            control_latch: None,
            read_buffer: 0,
            status: 0,
            line_interrupt_pending: false,
            line_counter: 0,
            frame_pixels: vec![0; PIXELS_PER_LINE * ACTIVE_LINES as usize],
            finished_frame: None,
        }
    }

    /// The /INT line as the CPU sees it: level, held until acknowledged
    /// by a status read.
    pub fn interrupt_asserted(&self) -> bool {
        let frame = self.status & status::FRAME_INTERRUPT != 0 && self.registers[1] & 0x20 != 0;
        let line = self.line_interrupt_pending && self.registers[0] & 0x10 != 0;
        frame || line
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        self.finished_frame.take()
    }

    pub fn line(&self) -> u16 {
        self.line
    }

    pub fn dot(&self) -> u16 {
        self.dot
    }

    /// Advance one dot; lines render and counters advance at line end.
    pub fn step_dot(&mut self) {
        self.dot += 1;
        if self.dot < DOTS_PER_LINE {
            return;
        }
        self.dot = 0;

        if self.line < ACTIVE_LINES {
            self.render_line(self.line);
        }

        // MacDonald's reload rule: the counter decrements on active lines
        // and the line just after; it reloads from R10 everywhere else.
        if self.line <= ACTIVE_LINES {
            if self.line_counter == 0 {
                self.line_counter = self.registers[10];
                self.line_interrupt_pending = true;
            } else {
                self.line_counter -= 1;
            }
        } else {
            self.line_counter = self.registers[10];
        }

        self.line += 1;
        if self.line == ACTIVE_LINES + 1 {
            self.status |= status::FRAME_INTERRUPT;
            self.finished_frame = Some(Frame {
                pixels: std::mem::replace(
                    &mut self.frame_pixels,
                    vec![0; PIXELS_PER_LINE * ACTIVE_LINES as usize],
                ),
                cram: self.cram,
            });
        }
        if self.line == LINES_PER_FRAME {
            self.line = 0;
        }
    }

    /// Backdrop-only line rendering; Mode 4 objects arrive next. A
    /// disabled display also shows the backdrop, so nothing gates yet.
    fn render_line(&mut self, line: u16) {
        let backdrop = 16 + (self.registers[7] & 0x0F);
        let start = line as usize * PIXELS_PER_LINE;
        self.frame_pixels[start..start + PIXELS_PER_LINE].fill(backdrop);
    }

    /// The V counter the CPU reads: NTSC 192-line counts $00-$DA then
    /// jumps to $D5-$FF.
    pub fn v_counter(&self) -> u8 {
        if self.line <= 0xDA {
            self.line as u8
        } else {
            (self.line - 6) as u8
        }
    }

    /// The H counter latches on TH transitions (the Light Phaser path);
    /// reads return the latched value. No latch source exists yet, so it
    /// reads the power-on value.
    pub fn h_counter(&self) -> u8 {
        0
    }

    pub fn read_status(&mut self) -> u8 {
        let value = self.status;
        self.status = 0;
        self.line_interrupt_pending = false;
        self.control_latch = None;
        value
    }

    /// Status without the read's acknowledge side effects.
    pub fn peek_status(&self) -> u8 {
        self.status
    }

    pub fn write_control(&mut self, value: u8) {
        match self.control_latch.take() {
            None => self.control_latch = Some(value),
            Some(low) => {
                self.address = u16::from_le_bytes([low, value & 0x3F]);
                self.code = value >> 6;
                if self.code == 0 {
                    self.read_buffer = self.vram[self.address as usize];
                    self.address = (self.address + 1) & 0x3FFF;
                } else if self.code == 2 {
                    let register = (value & 0x0F) as usize;
                    if register < self.registers.len() {
                        self.registers[register] = low;
                    }
                }
            }
        }
    }

    pub fn read_data(&mut self) -> u8 {
        self.control_latch = None;
        let value = self.read_buffer;
        self.read_buffer = self.vram[self.address as usize];
        self.address = (self.address + 1) & 0x3FFF;
        value
    }

    /// Data reads have buffer/address side effects; this has none.
    pub fn peek_data(&self) -> u8 {
        self.read_buffer
    }

    pub fn write_data(&mut self, value: u8) {
        self.control_latch = None;
        self.read_buffer = value;
        if self.code == 3 {
            self.cram[(self.address & 0x1F) as usize] = value;
        } else {
            self.vram[self.address as usize] = value;
        }
        self.address = (self.address + 1) & 0x3FFF;
    }
}
