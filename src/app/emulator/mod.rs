use std::time::Duration;

use iced::{
    Element,
    Length::{self, Fill},
    Subscription, Task, time,
    widget::{container, responsive, shader},
};

use crate::{
    app,
    game_boy::{GameBoy, joypad::Button, video::screen::Screen},
};

pub struct Emulator {
    game_boy: GameBoy,
    screen: Screen,
    running: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    EmulateFrame,
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Emulator(self)
    }
}

impl Emulator {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            game_boy,
            screen: Screen::new(),
            running: false,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        &self.game_boy
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        &mut self.game_boy
    }

    pub fn enable_debugger(self) -> app::debugger::Debugger {
        app::debugger::Debugger::new(self.game_boy)
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::EmulateFrame => {
                while !self.game_boy.step() {}
                self.screen = self.game_boy.screen().clone();
            }
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        responsive(|size| {
            let shortest = size.width.min(size.height);

            container(
                shader(&self.screen)
                    .width(Length::Fixed(shortest))
                    .height(Length::Fixed(shortest)),
            )
            .center(Fill)
            .into()
        })
        .into()
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn run(&mut self) {
        self.running = true;
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    pub fn reset(&mut self) {
        self.game_boy.reset();
    }

    pub fn press_button(&mut self, button: Button) {
        self.game_boy.press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.game_boy.release_button(button);
    }

    pub fn subscription(&self) -> Subscription<app::Message> {
        if self.running {
            Subscription::batch([
                time::every(Duration::from_micros(16740)).map(|_| Message::EmulateFrame.into())
            ])
        } else {
            Subscription::none()
        }
    }
}
