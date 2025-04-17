use std::collections::BTreeSet;

use crate::emulator::{GameBoy, cpu::instructions::Instruction, video::screen::Screen};
use instructions::InstructionsIterator;

pub mod instructions;

pub struct Debugger {
    game_boy: GameBoy,
    breakpoints: BTreeSet<u16>,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            game_boy,
            breakpoints: BTreeSet::new(),
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        &self.game_boy
    }

    pub fn step(&mut self) -> Option<Screen> {
        if self.game_boy.step() {
            Some(self.game_boy.screen().clone())
        } else {
            None
        }
    }

    pub fn step_over(&mut self) -> Option<Screen> {
        let mut it = InstructionsIterator::new(
            self.game_boy.cpu().program_counter,
            self.game_boy.memory_mapped(),
        );
        Instruction::decode(&mut it);
        let next_address = it.address.unwrap();

        if self.breakpoints.contains(&next_address) {
            self.run()
        } else {
            self.breakpoints.insert(next_address);
            let screen = self.run();
            self.breakpoints.remove(&next_address);
            screen
        }
    }

    pub fn step_frame(&mut self) -> Screen {
        while !self.game_boy.step() {}
        self.game_boy.screen().clone()
    }

    pub fn run(&mut self) -> Option<Screen> {
        let mut screen = None;

        while !self
            .breakpoints
            .contains(&self.game_boy.cpu().program_counter)
        {
            if let Some(new_screen) = self.step() {
                screen = Some(new_screen);
            }
        }

        screen
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
}
