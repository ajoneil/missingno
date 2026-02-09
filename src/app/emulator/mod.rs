use std::time::Duration;

use iced::{
    Element,
    Length::{self, Fill},
    Subscription, Task, time,
    widget::{button, container, mouse_area, responsive, shader, stack, svg},
};

use crate::{
    app::{
        self,
        core::icons::{self, Icon},
        screen::ScreenView,
    },
    game_boy::{GameBoy, joypad::Button, sgb::MaskMode, video::palette::PaletteChoice},
};

pub struct Emulator {
    game_boy: GameBoy,
    screen_view: ScreenView,
    running: bool,
    screen_hovered: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    EmulateFrame,
    ScreenHovered,
    ScreenUnhovered,
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
            screen_view: ScreenView::new(),
            running: false,
            screen_hovered: false,
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
                let freeze = self
                    .game_boy
                    .sgb()
                    .map(|sgb| sgb.mask_mode == MaskMode::Freeze)
                    .unwrap_or(false);
                if !freeze {
                    self.screen_view.screen = *self.game_boy.screen();
                }
                self.screen_view.sgb_render_data = self.game_boy.sgb().map(|sgb| sgb.render_data());
            }
            Message::ScreenHovered => self.screen_hovered = true,
            Message::ScreenUnhovered => self.screen_hovered = false,
        }

        Task::none()
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.screen_view.palette = palette;
    }

    pub fn view(&self, fullscreen: bool) -> Element<'_, app::Message> {
        let screen: Element<'_, app::Message> = responsive(|size| {
            let shortest = size.width.min(size.height);

            container(
                shader(&self.screen_view)
                    .width(Length::Fixed(shortest))
                    .height(Length::Fixed(shortest)),
            )
            .center(Fill)
            .into()
        })
        .into();

        if fullscreen {
            screen
        } else {
            let screen_stack = if self.screen_hovered {
                stack![
                    screen,
                    container(
                        button(icons::m(Icon::Fullscreen).style(|_, _| svg::Style {
                            color: Some(iced::Color::WHITE),
                        }))
                        .style(button::text)
                        .on_press(app::Message::ToggleFullscreen)
                    )
                    .align_right(Fill)
                    .padding(8)
                ]
                .into()
            } else {
                screen
            };

            mouse_area(screen_stack)
                .on_enter(Message::ScreenHovered.into())
                .on_exit(Message::ScreenUnhovered.into())
                .into()
        }
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
