use crate::{
    cpu::{Cycles, Interrupts},
    mmu::Mmu,
    timers::cycle_timer::CycleTimer,
};

pub struct Video {
    lcdc: u8,
    stat: u8,
    scroll_x: u8,
    scroll_y: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,

    vram: [u8; 0x2000],
    oam: [u8; 0xa0],

    dma_transfer_timer: Option<CycleTimer>,

    window: Window,

    state: State,
}

enum State {
    VBlank {
        timer: CycleTimer,
    },
    Render {
        line: u8,
        line_timer: CycleTimer,
        state: RenderState,
    },
}

enum RenderState {
    OAM,
    Pixel(u8),
    HBlank,
}

impl Video {
    const OAM_TIME: Cycles = Cycles(320);
    const LINE_TIME: Cycles = Cycles(1824);
    const VBLANK_TIME: Cycles = Cycles(18240);
    const RESOLUTION_X: u8 = 160;
    const RESOLUTION_Y: u8 = 144;

    pub fn new() -> Video {
        Video {
            lcdc: 0x91,
            stat: 0,
            scroll_x: 0,
            scroll_y: 0,
            lyc: 0,
            bgp: 0xfc,
            obp0: 0xff,
            obp1: 0xff,

            vram: [0; 0x2000],
            oam: [0; 0xa0],
            dma_transfer_timer: None,

            window: Window::new(),

            state: State::Render {
                line: 0,
                line_timer: CycleTimer::new(Self::LINE_TIME),
                state: RenderState::OAM,
            },
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0x8000..=0x9fff => self.read_vram(address),
            0xfe00..=0xfe9f => self.read_oam(address),
            0xfea0..=0xfeff => {
                if self.oam_accessible() {
                    0x00
                } else {
                    // On real hardware reading here will trigger OAM corruption
                    0xff
                }
            }
            0xff40 => self.lcdc,
            0xff41 => self.stat,
            0xff42 => self.scroll_y,
            0xff43 => self.scroll_x,
            0xff44 => match &self.state {
                State::Render { line, .. } => *line,
                State::VBlank { timer } => {
                    Self::RESOLUTION_Y + (timer.counted().0 / Self::LINE_TIME.0) as u8
                }
            },
            0xff45 => self.lyc,
            0xff47 => self.bgp,
            0xff48 => self.obp0,
            0xff49 => self.obp1,
            0xff4a => self.window.y,
            0xff4b => self.window.x,
            _ => panic!("Unimplemented video read from {:x}", address),
        }
    }

    pub fn write(&mut self, address: u16, val: u8, mmu: &Mmu) {
        match address {
            0x8000..=0x9fff => self.write_vram(address, val),
            0xfe00..=0xfe9f => self.write_oam(address, val),
            0xfea0..=0xfeff => (),
            0xff40 => self.lcdc = val,
            0xff41 => self.stat = val,
            0xff42 => self.scroll_y = val,
            0xff43 => self.scroll_x = val,
            0xff45 => self.lyc = val,
            0xff46 => self.begin_dma_transfer(val, mmu),
            0xff47 => self.bgp = val,
            0xff48 => self.obp0 = val,
            0xff49 => self.obp1 = val,
            0xff4a => self.window.y = val,
            0xff4b => self.window.x = val,
            _ => panic!("Unimplemented video write to {:x}", address),
        }
    }

    fn begin_dma_transfer(&mut self, address: u8, mmu: &Mmu) {
        let start_address = address as u16 * 0x100;

        println!("Beginning DMA transfer from {:4x}", start_address);

        for i in 0..=0x9f {
            self.oam[i] = mmu.read(start_address + i as u16, &self)
        }

        self.dma_transfer_timer = Some(CycleTimer::new(Cycles(580)));
    }

    pub fn dma_transfer_in_progess(&self) -> bool {
        self.dma_transfer_timer.is_some()
    }

    fn vram_accessible(&self) -> bool {
        match &self.state {
            State::VBlank { .. } => true,
            State::Render { state, .. } => match state {
                RenderState::OAM | RenderState::HBlank => true,
                RenderState::Pixel(_) => false,
            },
        }
    }

    fn read_vram(&self, address: u16) -> u8 {
        if self.vram_accessible() {
            self.vram[address as usize - 0x8000]
        } else {
            0xff
        }
    }

    fn write_vram(&mut self, address: u16, val: u8) {
        if self.vram_accessible() {
            self.vram[address as usize - 0x8000] = val
        }
    }

    fn oam_accessible(&self) -> bool {
        match &self.state {
            State::VBlank { .. } => true,
            State::Render { state, .. } => match state {
                RenderState::HBlank => true,
                RenderState::OAM | RenderState::Pixel(_) => false,
            },
        }
    }

    fn read_oam(&self, address: u16) -> u8 {
        if self.oam_accessible() {
            self.oam[address as usize - 0xfe00]
        } else {
            0xff
        }
    }

    fn write_oam(&mut self, address: u16, val: u8) {
        if self.oam_accessible() {
            self.oam[address as usize - 0xfe00] = val
        }
    }

    pub fn step(&mut self, cycles: Cycles, mmu: &mut Mmu) {
        if let Some(dma_transfer_timer) = &mut self.dma_transfer_timer {
            dma_transfer_timer.tick(cycles);
            if dma_transfer_timer.finished() {
                println!("DMA transfer complete!");
                self.dma_transfer_timer = None;
            }
        }

        let mut cycles_left = cycles;

        while cycles_left > Cycles(0) {
            cycles_left = match &mut self.state {
                State::VBlank { timer } => {
                    timer.tick(cycles_left);
                    if timer.finished() {
                        let overflow = timer.overflow();
                        println!("Beginning render");
                        self.state = State::Render {
                            line: 0,
                            line_timer: CycleTimer::new(Self::LINE_TIME),
                            state: RenderState::OAM,
                        };

                        overflow.unwrap_or(Cycles(0))
                    } else {
                        Cycles(0)
                    }
                }
                State::Render {
                    line,
                    line_timer,
                    state,
                } => {
                    line_timer.tick(Cycles(4));

                    match state {
                        RenderState::OAM => {
                            if line_timer.counted() == Self::OAM_TIME {
                                *state = RenderState::Pixel(0)
                            }
                        }
                        RenderState::Pixel(pixel) => {
                            // draw pixel here, then..

                            if (*pixel + 1) == Self::RESOLUTION_X {
                                *state = RenderState::HBlank {}
                            } else {
                                *pixel += 1
                            }
                        }
                        RenderState::HBlank => {
                            if line_timer.finished() {
                                if (*line + 1) == Self::RESOLUTION_Y {
                                    self.state = State::VBlank {
                                        timer: CycleTimer::new(Self::VBLANK_TIME),
                                    };
                                    println!(
                                        "Entering vblank, enabled interrupts {:?}",
                                        mmu.enabled_interrupts()
                                    );
                                    mmu.set_interrupt_flag(Interrupts::VBLANK)
                                } else {
                                    *state = RenderState::OAM;
                                    *line += 1;
                                    line_timer.reset();
                                }
                            }
                        }
                    }

                    cycles_left - Cycles(4)
                }
            }
        }
    }
}

struct Window {
    pub x: u8,
    pub y: u8,
}

impl Window {
    pub fn new() -> Self {
        Self { x: 0, y: 0 }
    }
}
