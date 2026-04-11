use std::time::Duration;

use iced::{
    Element,
    Length::{self, Fill},
    Subscription, Task, time,
    widget::{button, container, mouse_area, responsive, shader, stack, svg},
};

use crate::app::{
    self,
    ui::{
        icons::{self, Icon},
        sizes::border_s,
    },
    screen::{GameBoyScreen, ScreenDisplay, ScreenView, SgbScreen},
};
use missingno_gb::{GameBoy, joypad::Button, ppu::types::palette::PaletteChoice, sgb::MaskMode};

/// Frames of silence before we flush an SRAM save.
/// Games often write SRAM across several consecutive frames during a save
/// operation. We wait for writes to stop before persisting.
const SRAM_DEBOUNCE_FRAMES: u32 = 30; // ~0.5 seconds at 60fps

pub struct Emulator {
    game_boy: GameBoy,
    screen_view: ScreenView,
    running: bool,
    screen_hovered: bool,
    use_sgb_colors: bool,
    /// Countdown: frames since last SRAM write. When this reaches
    /// SRAM_DEBOUNCE_FRAMES, we fire SaveBattery. None = no pending save.
    sram_save_countdown: Option<u32>,
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
    pub fn new(game_boy: GameBoy, use_sgb_colors: bool) -> Self {
        Self {
            game_boy,
            screen_view: ScreenView::new(),
            running: false,
            screen_hovered: false,
            use_sgb_colors,
            sram_save_countdown: None,
        }
    }

    pub fn from_debugger(game_boy: GameBoy, screen_view: ScreenView, use_sgb_colors: bool) -> Self {
        Self {
            game_boy,
            screen_view,
            running: false,
            screen_hovered: false,
            use_sgb_colors,
            sram_save_countdown: None,
        }
    }

    pub fn set_use_sgb_colors(&mut self, use_sgb: bool) {
        self.use_sgb_colors = use_sgb;
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
                let screen = self.game_boy.screen().clone();
                let video_enabled = self.game_boy.ppu().control().video_enabled();
                let display = if let Some(sgb) = self.game_boy.sgb() {
                    let render_data = sgb.render_data(video_enabled);
                    if !video_enabled || sgb.mask_mode == MaskMode::Freeze {
                        ScreenDisplay::Sgb(SgbScreen::Freeze(render_data))
                    } else {
                        ScreenDisplay::Sgb(SgbScreen::Display(screen, render_data))
                    }
                } else if !video_enabled {
                    ScreenDisplay::GameBoy(GameBoyScreen::Off)
                } else {
                    ScreenDisplay::GameBoy(GameBoyScreen::Display(screen))
                };
                self.screen_view.use_sgb_colors = self.use_sgb_colors;
                self.screen_view.apply(display);

                // Debounce SRAM saves: reset countdown on each dirty frame,
                // fire SaveBattery after SRAM_DEBOUNCE_FRAMES of quiet.
                if sram_dirty {
                    self.sram_save_countdown = Some(0);
                } else if let Some(count) = &mut self.sram_save_countdown {
                    *count += 1;
                    if *count >= SRAM_DEBOUNCE_FRAMES {
                        self.sram_save_countdown = None;
                        return Task::done(app::Message::SaveBattery);
                    }
                }
            }
            Message::ScreenHovered => self.screen_hovered = true,
            Message::ScreenUnhovered => self.screen_hovered = false,
        }

        Task::none()
    }

    /// Force-flush any pending SRAM save. Call when pausing or closing.
    /// Returns true if there was a pending save.
    pub fn flush_pending_save(&mut self) -> bool {
        self.sram_save_countdown.take().is_some()
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
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, bg_alpha).into()),
                        text_color: iced::Color::WHITE,
                        border: Border::default().rounded(border_s()),
                        ..Default::default()
                    }
                }

                stack![
                    screen,
                    container(
                        button(icons::m(Icon::Expand).style(|_, _| svg::Style {
                            color: Some(iced::Color::WHITE),
                        }))
                        .style(overlay_button_style)
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
