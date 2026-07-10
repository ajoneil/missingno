use iced::{
    Element,
    Length::{self, Fill},
    Task,
    widget::{button, container, mouse_area, responsive, row, shader, stack, svg, text},
};

use crate::app::system::{ConsoleSwitch, ControlId, ControlInput};
use crate::app::{
    self,
    screen::{ScreenDisplay, ScreenView},
    system::SystemConsole,
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
    console: Option<Box<dyn SystemConsole>>,
    screen_view: ScreenView,
    running: bool,
    screen_hovered: bool,
    use_sgb_colors: bool,
    frame_blending: bool,
    /// The family's latching console switches and their current levels,
    /// captured at load so the overlay renders while the console is on the
    /// emu thread. Empty for families with none.
    switches: &'static [ConsoleSwitch],
    switch_levels: Vec<bool>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ScreenHovered,
    ScreenUnhovered,
    ToggleSwitch(usize),
}

impl From<Message> for app::Message {
    fn from(value: Message) -> Self {
        app::Message::Emulator(value)
    }
}

impl Emulator {
    pub fn new(
        console: Box<dyn SystemConsole>,
        use_sgb_colors: bool,
        frame_blending: bool,
    ) -> Self {
        let switches = console.console_switches();
        Self {
            console: Some(console),
            screen_view: ScreenView::new(),
            running: false,
            screen_hovered: false,
            use_sgb_colors,
            frame_blending,
            switches,
            switch_levels: switches.iter().map(|s| s.default_high).collect(),
        }
    }

    pub fn from_debugger(
        console: Box<dyn SystemConsole>,
        screen_view: ScreenView,
        use_sgb_colors: bool,
        frame_blending: bool,
    ) -> Self {
        let switches = console.console_switches();
        Self {
            console: Some(console),
            screen_view,
            running: false,
            screen_hovered: false,
            use_sgb_colors,
            frame_blending,
            switches,
            switch_levels: switches.iter().map(|s| s.default_high).collect(),
        }
    }

    pub fn set_use_sgb_colors(&mut self, use_sgb: bool) {
        self.use_sgb_colors = use_sgb;
    }

    pub fn set_frame_blending(&mut self, blend: bool) {
        self.frame_blending = blend;
    }

    /// The console, present only while paused/idle (not while running).
    pub fn console(&self) -> Option<&dyn SystemConsole> {
        self.console.as_deref()
    }

    /// Take the console to hand it to the emu thread for running.
    pub fn take_console(&mut self) -> Option<Box<dyn SystemConsole>> {
        self.console.take()
    }

    /// Put the console back when the emu thread returns it on pause.
    pub fn restore_console(&mut self, console: Box<dyn SystemConsole>) {
        self.console = Some(console);
    }

    /// Update the displayed frame from the emu thread's latest-frame slot.
    pub fn apply_frame(&mut self, display: ScreenDisplay) {
        self.screen_view.use_sgb_colors = self.use_sgb_colors;
        self.screen_view.blend = self.frame_blending;
        self.screen_view.apply(display);
    }

    /// Switch to debugger mode; systems without a debugger backend come
    /// back unchanged.
    pub fn enable_debugger(self) -> Result<app::debugger::Debugger, Box<Emulator>> {
        let use_sgb_colors = self.use_sgb_colors;
        let frame_blending = self.frame_blending;
        let console = self
            .console
            .expect("console present when enabling the debugger");
        app::debugger::Debugger::from_console(console, self.screen_view).map_err(|returned| {
            let (console, screen_view) = *returned;
            Box::new(Emulator::from_debugger(
                console,
                screen_view,
                use_sgb_colors,
                frame_blending,
            ))
        })
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::ScreenHovered => self.screen_hovered = true,
            Message::ScreenUnhovered => self.screen_hovered = false,
            Message::ToggleSwitch(index) => {
                if let (Some(level), Some(switch)) =
                    (self.switch_levels.get_mut(index), self.switches.get(index))
                {
                    *level = !*level;
                    // Route through the shared control path so it reaches
                    // the console whether it is local or on the emu thread.
                    return Task::done(app::Message::SetControl(switch.control.0, *level));
                }
            }
        }
        Task::none()
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.screen_view.palette = palette;
    }

    pub fn view(&self, fullscreen: bool) -> Element<'_, app::Message> {
        let screen: Element<'_, app::Message> = responsive(|size| {
            let (width, height) = self.screen_view.fitted_size(size);

            container(
                mouse_area(
                    shader(&self.screen_view)
                        .width(Length::Fixed(width))
                        .height(Length::Fixed(height)),
                )
                // Horizontal position over the screen drives the first
                // analog control (the VCS paddle); digital-only systems
                // ignore the axis.
                .on_move(move |point| app::Message::SetAxis(8, (point.x / width).clamp(0.0, 1.0))),
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

                let mut layers = stack![
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
                ];

                // The family's latching console switches (2600 difficulty /
                // TV type), top-left; each button flips its position.
                if !self.switches.is_empty() {
                    let mut switch_row = row![].spacing(8);
                    for (index, switch) in self.switches.iter().enumerate() {
                        let level = self
                            .switch_levels
                            .get(index)
                            .copied()
                            .unwrap_or(switch.default_high);
                        let label = format!("{}: {}", switch.label, switch.positions[level as usize]);
                        switch_row = switch_row.push(
                            button(text(label).size(12))
                                .style(overlay_button_style)
                                .on_press(Message::ToggleSwitch(index).into()),
                        );
                    }
                    layers = layers.push(container(switch_row).width(Fill).padding(8));
                }

                layers.into()
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

    pub fn set_control(&mut self, control: ControlId, input: ControlInput) {
        if let Some(console) = &mut self.console {
            console.set_control(control, input);
        }
    }
}
