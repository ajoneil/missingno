use std::collections::BTreeSet;

use crate::game_boy::{
    BusAccess, BusAccessKind, GameBoy, cpu::instructions::Instruction, ppu::screen::Screen,
};
use instructions::InstructionsIterator;

pub mod instructions;

pub struct Debugger {
    game_boy: GameBoy,
    breakpoints: BTreeSet<u16>,
    watchpoints_read: BTreeSet<u16>,
    watchpoints_write: BTreeSet<u16>,
    last_watchpoint_hit: Option<BusAccess>,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            game_boy,
            breakpoints: BTreeSet::new(),
            watchpoints_read: BTreeSet::new(),
            watchpoints_write: BTreeSet::new(),
            last_watchpoint_hit: None,
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

    pub fn step(&mut self) -> Option<Screen> {
        if self.game_boy.step() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_dot(&mut self) -> Option<Screen> {
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

    fn has_watchpoints(&self) -> bool {
        !self.watchpoints_read.is_empty() || !self.watchpoints_write.is_empty()
    }

    pub fn step_frame(&mut self) -> Option<Screen> {
        self.last_watchpoint_hit = None;
        if self.has_watchpoints() {
            self.step_frame_watched()
        } else {
            self.step_frame_simple()
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
        loop {
            let (new_screen, trace) = self.game_boy.step_traced(true);
            let screen = if new_screen {
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

    fn breakpoint_triggered(&self) -> bool {
        self.breakpoints
            .contains(&self.game_boy.cpu().program_counter)
    }

    fn check_watchpoints(&self, trace: &[BusAccess]) -> Option<BusAccess> {
        for access in trace {
            match access.kind {
                BusAccessKind::Read if self.watchpoints_read.contains(&access.address) => {
                    return Some(*access);
                }
                BusAccessKind::Write if self.watchpoints_write.contains(&access.address) => {
                    return Some(*access);
                }
                _ => {}
            }
        }
        None
    }

    pub fn last_watchpoint_hit(&self) -> Option<&BusAccess> {
        self.last_watchpoint_hit.as_ref()
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
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

    pub fn watchpoints_read(&self) -> &BTreeSet<u16> {
        &self.watchpoints_read
    }

    pub fn watchpoints_write(&self) -> &BTreeSet<u16> {
        &self.watchpoints_write
    }

    pub fn set_watchpoint_read(&mut self, address: u16) {
        self.watchpoints_read.insert(address);
    }

    pub fn set_watchpoint_write(&mut self, address: u16) {
        self.watchpoints_write.insert(address);
    }

    pub fn clear_watchpoint_read(&mut self, address: u16) {
        self.watchpoints_read.remove(&address);
    }

    pub fn clear_watchpoint_write(&mut self, address: u16) {
        self.watchpoints_write.remove(&address);
    }
}
