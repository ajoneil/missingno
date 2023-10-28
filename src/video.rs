use crate::cpu::Cycles;

pub struct Video {
    lcdc: u8,
    stat: u8,
    scroll_x: u8,
    scroll_y: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,

    state: State,
}

struct Timer {
    length: Cycles,
    counted: Cycles,
}

impl Timer {
    pub fn new(length: Cycles) -> Self {
        Self {
            length,
            counted: Cycles(0),
        }
    }

    pub fn tick(&mut self, delta: Cycles) {
        self.counted += delta;
    }

    pub fn counted(&self) -> Cycles {
        self.counted
    }

    pub fn finished(&self) -> bool {
        self.counted >= self.length
    }

    pub fn reset(&mut self) {
        self.counted = Cycles(0)
    }

    pub fn overflow(&self) -> Option<Cycles> {
        if self.counted > self.length {
            Some(self.counted - self.length)
        } else {
            None
        }
    }
}

enum State {
    VBlank {
        timer: Timer,
    },
    Render {
        line: u8,
        line_timer: Timer,
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
            ly: 0,
            lyc: 0,
            bgp: 0xfc,
            obp0: 0xff,
            obp1: 0xff,

            state: State::Render {
                line: 0,
                line_timer: Timer::new(Self::LINE_TIME),
                state: RenderState::OAM,
            },
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xff40 => self.lcdc,
            0xff41 => self.stat,
            0xff42 => self.scroll_y,
            0xff43 => self.scroll_x,
            0xff44 => self.ly,
            0xff45 => self.lyc,
            0xff47 => self.bgp,
            0xff48 => self.obp0,
            0xff49 => self.obp1,
            _ => panic!("Unimplemented video read from {:x}", address),
        }
    }

    pub fn write(&mut self, address: u16, val: u8) {
        match address {
            0xff40 => self.lcdc = val,
            0xff41 => self.stat = val,
            0xff42 => self.scroll_y = val,
            0xff43 => self.scroll_x = val,
            0xff45 => self.lyc = val,
            0xff47 => self.bgp = val,
            0xff48 => self.obp0 = val,
            0xff49 => self.obp1 = val,
            _ => panic!("Unimplemented video write to {:x}", address),
        }
    }

    pub fn step(&mut self, cycles: Cycles) {
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
                            line_timer: Timer::new(Self::LINE_TIME),
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
                                        timer: Timer::new(Self::VBLANK_TIME),
                                    };
                                    println!("Entering vblank");
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
