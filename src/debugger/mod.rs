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
        loop {
            let screen = self.step();

            if screen.is_some() || self.breakpoint_triggered() {
                return screen;
            }
        }
    }

    fn breakpoint_triggered(&self) -> bool {
        self.breakpoints
            .contains(&self.game_boy.cpu().program_counter)
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
