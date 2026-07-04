use iced::{
    Element,
    Length::{self, Fill},
    Task,
    widget::{button, container, mouse_area, responsive, shader, stack, svg},
};

use crate::app::{
    self,
    console::AnyConsole,
    screen::{ScreenDisplay, ScreenView},
    ui::{
        icons::{self, Icon},
        sizes::border_s,
    },
};
use missingno_gb::ppu::types::palette::PaletteChoice;

/// The UI-side shell for a plain (non-debugger) game. While the game runs the
/// console lives on the emu thread (`console` is `None`); it is recovered here
/// synchronously on pause so all inspection paths keep working.
pub struct Emulator {
    console: Option<AnyConsole>,
    screen_view: ScreenView,
    running: bool,
    screen_hovered: bool,
    use_sgb_colors: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    ScreenHovered,
    ScreenUnhovered,
}

impl From<Message> for app::Message {
    fn from(value: Message) -> Self {
        app::Message::Emulator(value)
    }
}

impl Emulator {
    pub fn new(console: AnyConsole, use_sgb_colors: bool) -> Self {
        Self {
            console: Some(console),
            screen_view: ScreenView::new(),
            running: false,
            screen_hovered: false,
            use_sgb_colors,
        }
    }

    pub fn from_debugger(
        console: AnyConsole,
        screen_view: ScreenView,
        use_sgb_colors: bool,
    ) -> Self {
        Self {
            console: Some(console),
            screen_view,
            running: false,
            screen_hovered: false,
            use_sgb_colors,
        }
    }

    pub fn set_use_sgb_colors(&mut self, use_sgb: bool) {
        self.use_sgb_colors = use_sgb;
    }

    /// The console, present only while paused/idle (not while running).
    pub fn console(&self) -> Option<&AnyConsole> {
        self.console.as_ref()
    }

    /// Take the console to hand it to the emu thread for running.
    pub fn take_console(&mut self) -> Option<AnyConsole> {
        self.console.take()
    }

    /// Put the console back when the emu thread returns it on pause.
    pub fn restore_console(&mut self, console: AnyConsole) {
        self.console = Some(console);
    }

    /// Update the displayed frame from the emu thread's latest-frame slot.
    pub fn apply_frame(&mut self, display: ScreenDisplay) {
        self.screen_view.use_sgb_colors = self.use_sgb_colors;
        self.screen_view.apply(display);
    }

    pub fn enable_debugger(self) -> app::debugger::AnyDebugger {
        let console = self
            .console
            .expect("console present when enabling the debugger");
        app::debugger::AnyDebugger::from_emulator(console, self.screen_view)
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
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

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn reset(&mut self) {
        if let Some(console) = &mut self.console {
            console.reset();
        }
    }

    pub fn press_button(&mut self, button: missingno_gb::joypad::Button) {
        if let Some(console) = &mut self.console {
            console.press_button(button);
        }
    }

    pub fn release_button(&mut self, button: missingno_gb::joypad::Button) {
        if let Some(console) = &mut self.console {
            console.release_button(button);
        }
    }
}
