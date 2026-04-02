use std::time::Duration;

use iced::{
    Element,
    Length::{self, Fill},
    Subscription, Task, time,
    widget::{button, container, mouse_area, responsive, shader, stack, svg},
};

use crate::app::{
    self,
    core::icons::{self, Icon},
    screen::{GameBoyScreen, ScreenView, SgbScreen},
};
use missingno_gb::{GameBoy, joypad::Button, ppu::types::palette::PaletteChoice, sgb::MaskMode};

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

    pub fn from_debugger(game_boy: GameBoy, screen_view: ScreenView) -> Self {
        Self {
            game_boy,
            screen_view,
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
        app::debugger::Debugger::from_emulator(self.game_boy, self.screen_view)
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::EmulateFrame => {
                // A GB frame is ~70224 T-cycles. Allow 2x headroom to avoid
                // hanging the UI if the PPU never produces a frame (e.g. LCD off).
                const MAX_DOTS_PER_FRAME: u32 = 70224 * 2;
                let mut dots = 0;
                let mut sram_dirty = false;
                loop {
                    let result = self.game_boy.step();
                    dots += result.dots;
                    sram_dirty |= result.sram_dirty;
                    if result.new_screen || dots >= MAX_DOTS_PER_FRAME {
                        break;
                    }
                }
                if sram_dirty {
                    return Task::done(app::Message::SaveBattery);
                }
                let screen = self.game_boy.screen().clone();
                let video_enabled = self.game_boy.ppu().control().video_enabled();
                let display = if let Some(sgb) = self.game_boy.sgb() {
                    let render_data = sgb.render_data(video_enabled);
                    if sgb.mask_mode == MaskMode::Freeze {
                        SgbScreen::Freeze(render_data).into()
                    } else {
                        SgbScreen::Display(screen, render_data).into()
                    }
                } else if !video_enabled {
                    GameBoyScreen::Off.into()
                } else {
                    GameBoyScreen::Display(screen).into()
                };
                self.screen_view.apply(display);
            }
            Message::ScreenHovered => self.screen_hovered = true,
            Message::ScreenUnhovered => self.screen_hovered = false,
        }

        Task::none()
    }

    pub fn reset_hover(&mut self) {
        self.screen_hovered = false;
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.screen_view.palette = palette;
    }

    pub fn view(&self, fullscreen: bool, has_game_info: bool) -> Element<'_, app::Message> {
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
                use iced::widget::row;
                use iced::Border;

                fn overlay_button_style(
                    _theme: &iced::Theme,
                    status: button::Status,
                ) -> button::Style {
                    let bg_alpha = match status {
                        button::Status::Hovered => 0.6,
                        _ => 0.4,
                    };
                    button::Style {
                        background: Some(
                            iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into(),
                        ),
                        text_color: iced::Color::WHITE,
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }
                }

                let mut buttons = row![
                    button(icons::m(Icon::Expand).style(|_, _| svg::Style {
                        color: Some(iced::Color::WHITE),
                    }))
                    .style(overlay_button_style)
                    .on_press(app::Message::ToggleFullscreen)
                ]
                .spacing(4);

                if has_game_info {
                    buttons = buttons.push(
                        button(icons::m(Icon::Info).style(|_, _| svg::Style {
                            color: Some(iced::Color::WHITE),
                        }))
                        .style(overlay_button_style)
                        .on_press(app::Message::ToggleGameInfo),
                    );
                }

                stack![
                    screen,
                    container(buttons).align_right(Fill).padding(8)
                ]
                .into()
            } else {
                screen
            };

            mouse_area(screen_stack)
                .on_enter(Message::ScreenHovered.into())
                .on_exit(Message::ScreenUnhovered.into())
                .on_move(|_| Message::ScreenHovered.into())
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
