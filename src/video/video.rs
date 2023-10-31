use std::thread::current;

use super::{palette::Palette, tile::Tile};
use crate::{
    cpu::{Cycles, Interrupts},
    joypad::Joypad,
    mmu::Mmu,
    timers::{cycle_timer::CycleTimer, timers::Timers},
};
use bitflags::bitflags;
use rgb::RGBA8;

pub struct Video {
    vram: [u8; 0x2000],
    oam: [u8; 0xa0],

    display: [[RGBA8; Video::RESOLUTION_X as _]; Video::RESOLUTION_Y as _],

    dma_transfer_timer: Option<CycleTimer>,

    control: Control,
    stat_interrupts: StatInterruptCondition,
    stat: StatInterruptCondition,
    lcd_y_compare: u8,
    background: Background,
    window: Window,

    state: State,
    frame_ready: bool,
}

enum State {
    Disabled,
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
    RenderingLine,
    HBlank,
}

impl Video {
    const OAM_TIME: Cycles = Cycles(320);
    const LINE_TIME: Cycles = Cycles(1824);
    const MIN_LINE_RENDER_TIME: Cycles = Cycles(688);
    const VBLANK_TIME: Cycles = Cycles(18240);
    const RESOLUTION_X: u8 = 160;
    const RESOLUTION_Y: u8 = 144;

    pub fn new() -> Video {
        Video {
            vram: [0; 0x2000],
            oam: [0; 0xa0],
            display: [[Palette::MONOCHROME_GREEN.color(3); Self::RESOLUTION_X as _];
                Self::RESOLUTION_Y as _],
            dma_transfer_timer: None,

            control: Control::from_bits_retain(0x91),
            lcd_y_compare: 0,
            stat: StatInterruptCondition::empty(),
            stat_interrupts: StatInterruptCondition::empty(),
            background: Background::new(),
            window: Window::new(),

            state: State::Render {
                line: 0,
                line_timer: CycleTimer::new(Self::LINE_TIME),
                state: RenderState::OAM,
            },

            frame_ready: false,
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
            0xff40 => self.control.bits(),
            // 0xff41 => self.stat,
            0xff42 => self.background.y,
            0xff43 => self.background.x,
            0xff44 => self.lcd_y(),
            // 0xff45 => self.lyc,
            // 0xff47 => self.bgp,
            // 0xff48 => self.obp0,
            // 0xff49 => self.obp1,
            0xff4a => self.window.y,
            0xff4b => self.window.x,
            _ => panic!("Unimplemented video read from {:x}", address),
        }
    }

    pub fn write(
        &mut self,
        address: u16,
        val: u8,
        mmu: &mut Mmu,
        timers: &mut Timers,
        joypad: &mut Joypad,
    ) {
        match address {
            0x8000..=0x9fff => self.write_vram(address, val),
            0xfe00..=0xfe9f => self.write_oam(address, val),
            0xfea0..=0xfeff => {
                println!(
                    "attempt to write {:2x} to forbidden address {:4x}",
                    val, address
                )
            }
            // 0xfea0..=0xfeff => (),
            0xff40 => {
                self.control = Control::from_bits_retain(val);
                if !self.control.enabled() {
                    self.state = State::Disabled;
                    mmu.reset_interrupt_flag(Interrupts::VBLANK)
                } else if let State::Disabled = self.state {
                    self.state = State::Render {
                        line: 0,
                        line_timer: CycleTimer::new(Self::LINE_TIME),
                        state: RenderState::OAM,
                    }
                }
            }
            0xff41 => self.stat_interrupts = StatInterruptCondition::from_bits_truncate(val),
            0xff42 => self.background.y = val,
            0xff43 => self.background.x = val,
            0xff45 => {
                self.lcd_y_compare = val;
            }
            0xff46 => self.begin_dma_transfer(val, mmu, timers, joypad),
            0xff47 => {
                println!("bg palette control register unimplemented")
            }
            0xff48..=0xff49 => {
                println!("obj palette control registers unimplemented")
            }
            // 0xff49 => self.obp1 = val,
            0xff4a => self.window.y = val,
            0xff4b => self.window.x = val,
            _ => panic!("Unimplemented video write to {:x}", address),
        }
    }

    fn begin_dma_transfer(&mut self, address: u8, mmu: &Mmu, timers: &Timers, joypad: &Joypad) {
        let start_address = address as u16 * 0x100;

        println!("Beginning DMA transfer from {:4x}", start_address);

        for i in 0..=0x9f {
            self.oam[i] = mmu.read(start_address + i as u16, &self, timers, joypad)
        }

        self.dma_transfer_timer = Some(CycleTimer::new(Cycles(580)));
    }

    pub fn dma_transfer_in_progess(&self) -> bool {
        self.dma_transfer_timer.is_some()
    }

    pub fn frame_ready(&self) -> bool {
        self.frame_ready
    }

    pub fn take_frame(&mut self) {
        self.frame_ready = false
    }

    pub fn display(&self) -> &[[RGBA8; Video::RESOLUTION_X as _]; Video::RESOLUTION_Y as _] {
        &self.display
    }

    fn vram_accessible(&self) -> bool {
        match &self.state {
            State::VBlank { .. } => true,
            State::Render { state, .. } => match state {
                RenderState::OAM | RenderState::HBlank => true,
                RenderState::RenderingLine => false,
            },
            State::Disabled => true,
        }
    }

    fn read_vram(&self, address: u16) -> u8 {
        if self.vram_accessible() {
            self.vram[address as usize - 0x8000]
        } else {
            0xff
        }
    }

    pub fn all_tiles(&self) -> [Tile; 384] {
        (0..384)
            .map(|i| self.get_tile(0x8000 + (i * 16)))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    pub fn get_tile(&self, address: u16) -> Tile {
        let start = address as usize - 0x8000;
        Tile::new(self.vram[start..(start + 16)].try_into().unwrap())
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
                RenderState::OAM | RenderState::RenderingLine => false,
            },
            State::Disabled => true,
        }
    }

    fn lcd_y(&self) -> u8 {
        match &self.state {
            State::Render { line, .. } => *line,
            State::VBlank { timer } => {
                Self::RESOLUTION_Y as u8 + (timer.counted().0 / Self::LINE_TIME.0) as u8
            }
            State::Disabled => 0xff,
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
                State::Disabled => Cycles(0),
                State::VBlank { timer } => {
                    timer.tick(cycles_left);
                    if timer.finished() {
                        let overflow = timer.overflow();
                        println!("Beginning render {:?}", self.control);

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
                                *state = RenderState::RenderingLine
                            }
                        }
                        RenderState::RenderingLine => {
                            if line_timer.counted() >= (Self::OAM_TIME + Self::MIN_LINE_RENDER_TIME)
                            {
                                if self.control.background_enabled() {
                                    let base_tilemap_address = if self.control.window_enabled() {
                                        self.control.window_tilemap_address()
                                    } else {
                                        self.control.background_tilemap_address()
                                    };

                                    let tile_map_address = base_tilemap_address
                                        + ((self.background.y as u16 / 8) * 255)
                                        + self.background.x as u16 / 8;

                                    let tile_y = self.background.y % 8;
                                    let tile_x_offset = self.background.x % 8;
                                    let num_tiles: u8 = if tile_x_offset == 0 {
                                        (Self::RESOLUTION_X / 8) as _
                                    } else {
                                        (Self::RESOLUTION_X / 8 + 1) as _
                                    };

                                    let tiles_address = self.control.tile_data_address();

                                    for i in 0..num_tiles {
                                        let tile_index = self.vram
                                            [(tile_map_address + i as u16 - 0x8000) as usize];
                                        let start: usize =
                                            (tiles_address + tile_index as u16 - 0x8000) as _;
                                        let tile = Tile::new(
                                            self.vram[start..(start + 16)].try_into().unwrap(),
                                        );

                                        let tile_x_start: i32 = (i * 8 - tile_x_offset) as i32;
                                        let start_pixel =
                                            if tile_x_start >= 0 { 0 } else { tile_x_offset };
                                        let end_pixel =
                                            if tile_x_start + 7 < Self::RESOLUTION_X as _ {
                                                7
                                            } else {
                                                7 - tile_x_offset
                                            };

                                        for pixel in (start_pixel..=end_pixel) {
                                            self.display[*line as usize]
                                                [(pixel - tile_x_offset) as usize] = tile
                                                .pixel_color(
                                                    pixel,
                                                    tile_y,
                                                    &Palette::MONOCHROME_GREEN,
                                                )
                                        }
                                    }
                                }

                                *state = RenderState::HBlank {}
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
                                    mmu.set_interrupt_flag(Interrupts::VBLANK);
                                    self.frame_ready = true
                                } else {
                                    *state = RenderState::OAM;
                                    *line += 1;
                                    line_timer.reset();
                                }
                            }
                        }
                    }

                    let mut new_stat = StatInterruptCondition::empty();

                    if self.lcd_y() == self.lcd_y_compare {
                        new_stat.insert(StatInterruptCondition::LYC)
                    }

                    let new_flags = new_stat - self.stat;
                    self.stat = new_stat;

                    if !new_flags.intersection(self.stat_interrupts).is_empty() {
                        mmu.set_interrupt_flag(Interrupts::LCD)
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

struct Background {
    pub x: u8,
    pub y: u8,
}

impl Background {
    pub fn new() -> Self {
        Self { x: 0, y: 0 }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct StatInterruptCondition: u8 {
        const LYC    = 0b01000000;
        const OAM    = 0b00100000;
        const VBLANK = 0b00010000;
        const HBLANK = 0b00001000;
    }
}

bitflags! {
    #[derive(Debug)]
    pub struct Control: u8 {
        const MASTER_CONTROL        = 0b10000000;
        const WINDOW_TILEMAP_AREA   = 0b01000000;
        const WINDOW_ENABLED        = 0b00100000;
        const TILE_DATA_AREA        = 0b00010000;
        const BG_TILEMAP            = 0b00001000;
        const OBJ_SIZE              = 0b00000100;
        const OBJ_ENABLED           = 0b00000010;
        const BG_AND_WINDOW_ENABLED = 0b00000001;
    }
}

pub enum SpriteSize {
    SINGLE_TILE,
    DOUBLE_TILE,
}

impl Control {
    pub fn enabled(&self) -> bool {
        self.contains(Control::MASTER_CONTROL)
    }

    pub fn sprites_enabled(&self) -> bool {
        self.contains(Control::OBJ_ENABLED)
    }

    pub fn background_enabled(&self) -> bool {
        self.contains(Control::BG_AND_WINDOW_ENABLED)
    }

    pub fn window_enabled(&self) -> bool {
        self.contains(Control::WINDOW_ENABLED) && self.contains(Control::BG_AND_WINDOW_ENABLED)
    }

    pub fn window_tilemap_address(&self) -> u16 {
        if self.contains(Control::WINDOW_TILEMAP_AREA) {
            0x9c00
        } else {
            0x9800
        }
    }

    pub fn tile_data_address(&self) -> u16 {
        if self.contains(Control::TILE_DATA_AREA) {
            0x8000
        } else {
            0x8800
        }
    }

    pub fn background_tilemap_address(&self) -> u16 {
        if self.contains(Control::BG_TILEMAP) {
            0x9800
        } else {
            0x9c00
        }
    }

    pub fn sprite_size(&self) -> SpriteSize {
        if self.contains(Control::OBJ_SIZE) {
            SpriteSize::SINGLE_TILE
        } else {
            SpriteSize::DOUBLE_TILE
        }
    }
}
