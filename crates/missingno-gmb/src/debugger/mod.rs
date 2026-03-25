use std::collections::BTreeSet;

use crate::{
    BusAccess, BusAccessKind, GameBoy,
    cpu::instructions::Instruction,
    ppu::{self, rendering::Mode, screen::Screen},
};
use instructions::InstructionsIterator;

pub mod instructions;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpuRegister {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchCondition {
    BusRead { address: u16 },
    BusWrite { address: u16 },
    DmaRead { address: u16 },
    DmaWrite { address: u16 },
    Scanline(u8),
    PpuMode(Mode),
    PixelCounter(u8),
    PpuRegister { register: ppu::Register, value: u8 },
    CpuRegister { register: CpuRegister, value: u8 },
    All(Vec<WatchCondition>),
}

impl WatchCondition {
    fn needs_bus_trace(&self) -> bool {
        match self {
            WatchCondition::BusRead { .. }
            | WatchCondition::BusWrite { .. }
            | WatchCondition::DmaRead { .. }
            | WatchCondition::DmaWrite { .. } => true,
            WatchCondition::All(conditions) => conditions.iter().any(|c| c.needs_bus_trace()),
            _ => false,
        }
    }
}

pub struct Debugger {
    game_boy: GameBoy,
    breakpoints: BTreeSet<u16>,
    watchpoints: Vec<WatchCondition>,
    last_watchpoint_hit: Option<WatchCondition>,
    /// T-cycle counter. Increments once per dot. Not hardware state —
    /// debugging/tracing infrastructure built on top of the emulation core.
    dot_count: u64,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            game_boy,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_watchpoint_hit: None,
            dot_count: 0,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        &self.game_boy
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        &mut self.game_boy
    }

    pub fn game_boy_take(self) -> GameBoy {
        self.game_boy
    }

    pub fn dot_count(&self) -> u64 {
        self.dot_count
    }

    pub fn step(&mut self) -> Option<Screen> {
        let result = self.game_boy.step();
        self.dot_count += result.dots as u64;
        if result.new_screen {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_phase(&mut self) -> Option<Screen> {
        if self.game_boy.step_phase().new_screen {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_dot(&mut self) -> Option<Screen> {
        self.dot_count += 1;
        if self.game_boy.step_dot() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_over(&mut self) -> Option<Screen> {
        let mut it = InstructionsIterator::new(self.game_boy.cpu().program_counter, &self.game_boy);
        Instruction::decode(&mut it);
        let next_address = it.address.unwrap();

        let temp_breakpoint = if self.breakpoints.contains(&next_address) {
            None
        } else {
            Some(next_address)
        };

        let mut last_screen = None;

        loop {
            let screen = self.step_frame();
            match screen {
                Some(screen) => {
                    last_screen = Some(screen);
                }
                None => {
                    break;
                }
            }
        }

        if let Some(temp_breakpoint) = temp_breakpoint {
            self.breakpoints.remove(&temp_breakpoint);
        }

        last_screen
    }

    pub fn step_frame(&mut self) -> Option<Screen> {
        self.last_watchpoint_hit = None;
        if self.watchpoints.is_empty() {
            self.step_frame_simple()
        } else {
            self.step_frame_watched()
        }
    }

    fn step_frame_simple(&mut self) -> Option<Screen> {
        loop {
            let screen = self.step();
            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched(&mut self) -> Option<Screen> {
        if self.watchpoints.iter().any(|w| w.needs_bus_trace()) {
            self.step_frame_watched_traced()
        } else {
            self.step_frame_watched_dots()
        }
    }

    fn step_frame_watched_traced(&mut self) -> Option<Screen> {
        loop {
            let (result, trace) = self.game_boy.step_traced(true);
            self.dot_count += result.dots as u64;
            let screen = if result.new_screen {
                Some(self.game_boy.screen().clone())
            } else {
                None
            };

            if let Some(hit) = self.check_watchpoints(&trace) {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn step_frame_watched_dots(&mut self) -> Option<Screen> {
        loop {
            let screen = self.step_phase();

            if let Some(hit) = self.check_watchpoints(&[]) {
                self.last_watchpoint_hit = Some(hit);
                return screen;
            }

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn breakpoint_triggered(&self) -> bool {
        self.breakpoints
            .contains(&self.game_boy.cpu().program_counter)
    }

    fn check_watchpoints(&self, trace: &[BusAccess]) -> Option<WatchCondition> {
        for condition in &self.watchpoints {
            if self.condition_matches(condition, trace) {
                return Some(condition.clone());
            }
        }
        None
    }

    fn condition_matches(&self, condition: &WatchCondition, trace: &[BusAccess]) -> bool {
        let ppu = self.game_boy.ppu();
        let cpu = self.game_boy.cpu();

        match condition {
            WatchCondition::BusRead { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::Read && a.address == *address),
            WatchCondition::BusWrite { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::Write && a.address == *address),
            WatchCondition::DmaRead { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::DmaRead && a.address == *address),
            WatchCondition::DmaWrite { address } => trace
                .iter()
                .any(|a| a.kind == BusAccessKind::DmaWrite && a.address == *address),
            WatchCondition::Scanline(target) => {
                ppu.read_register(ppu::Register::CurrentScanline) == *target
            }
            WatchCondition::PpuMode(target) => ppu.mode() == *target,
            WatchCondition::PixelCounter(target) => ppu
                .pipeline_state()
                .is_some_and(|snap| snap.pixel_counter == *target),
            WatchCondition::PpuRegister { register, value } => {
                ppu.read_register(*register) == *value
            }
            WatchCondition::CpuRegister { register, value } => {
                let actual = match register {
                    CpuRegister::A => cpu.a,
                    CpuRegister::B => cpu.b,
                    CpuRegister::C => cpu.c,
                    CpuRegister::D => cpu.d,
                    CpuRegister::E => cpu.e,
                    CpuRegister::H => cpu.h,
                    CpuRegister::L => cpu.l,
                };
                actual == *value
            }
            WatchCondition::All(conditions) => {
                conditions.iter().all(|c| self.condition_matches(c, trace))
            }
        }
    }

    pub fn last_watchpoint_hit(&self) -> Option<&WatchCondition> {
        self.last_watchpoint_hit.as_ref()
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
        self.dot_count = 0;
    }

    pub fn breakpoints(&self) -> &BTreeSet<u16> {
        &self.breakpoints
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        self.breakpoints.insert(address);
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        self.breakpoints.remove(&address);
    }

    pub fn watchpoints(&self) -> &[WatchCondition] {
        &self.watchpoints
    }

    pub fn add_watchpoint(&mut self, condition: WatchCondition) {
        if !self.watchpoints.contains(&condition) {
            self.watchpoints.push(condition);
        }
    }

    pub fn remove_watchpoint(&mut self, condition: &WatchCondition) {
        self.watchpoints.retain(|w| w != condition);
    }

    pub fn clear_watchpoints(&mut self) {
        self.watchpoints.clear();
    }
}
