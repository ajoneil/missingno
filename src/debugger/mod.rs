use std::collections::BTreeSet;

use crate::emulator::{GameBoy, cpu::instructions::Instruction};
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

    pub fn step(&mut self) {
        self.game_boy.step();
    }

    pub fn step_over(&mut self) {
        let mut it = InstructionsIterator::new(
            self.game_boy.cpu().program_counter,
            self.game_boy.memory_mapped(),
        );
        Instruction::decode(&mut it);
        let next_address = it.address.unwrap();

        if self.breakpoints.contains(&next_address) {
            self.run();
        } else {
            self.breakpoints.insert(next_address);
            self.run();
            self.breakpoints.remove(&next_address);
        }
    }

    pub fn step_frame(&mut self) {
        while !self.game_boy.step() {}
    }

    pub fn run(&mut self) {
        while !self
            .breakpoints
            .contains(&self.game_boy.cpu().program_counter)
        {
            self.step();
        }
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
