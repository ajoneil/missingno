use iced::{
    Element,
    Length::{self, Fill},
    Task,
    widget::{button, container, mouse_area, responsive, row, shader, stack, svg},
};

use crate::app::system::{ConsoleSwitch, ControlId, ControlInput, Platform, gb};
use crate::app::{
    self,
    screen::{Frame, ScreenView},
    system::SystemConsole,
    ui::{
        icons::{self, Icon},
        sizes::border_s,
    },
};
use missingno_gb::ppu::types::palette::PaletteChoice;

mod panels;
pub use panels::{CaptureKind, PlayLogEntry, PlayPanel};

/// The UI-side shell for a plain (non-debugger) game. While the game runs the
/// console lives on the emu thread (`console` is `None`); it is recovered here
/// synchronously on pause so all inspection paths keep working.
pub struct Emulator {
    console: Option<Box<dyn SystemConsole>>,
    /// The platform this game presents, captured at load; carried so a
    /// debugger toggle can key its panes without the console at hand.
    platform: Platform,
    screen_view: ScreenView,
    running: bool,
    screen_hovered: bool,
    /// The user's monochrome palette choice, held so a palette change can rebuild
    /// the renderer's colour policy and the Display panel can show the selection.
    palette: PaletteChoice,
    use_sgb_colors: bool,
    persistence: bool,
    /// The family's latching console switches and their current levels,
    /// captured at load so the Console panel renders while the console is on
    /// the emu thread. Empty for families with none.
    switches: &'static [ConsoleSwitch],
    switch_levels: Vec<bool>,
    /// Whether this console has a selectable monochrome palette (DMG),
    /// captured at load; gates the Display panel.
    monochrome_palette: bool,
    /// The play-mode side panels currently open (rendered stacked, in a
    /// stable order regardless of toggle order).
    open_panels: Vec<PlayPanel>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ScreenHovered,
    ScreenUnhovered,
    ToggleSwitch(usize),
    TogglePanel(PlayPanel),
}

impl From<Message> for app::Message {
    fn from(value: Message) -> Self {
        app::Message::Emulator(value)
    }
}

impl Emulator {
    pub fn new(
        console: Box<dyn SystemConsole>,
        platform: Platform,
        use_sgb_colors: bool,
        persistence: bool,
    ) -> Self {
        let switches = console.console_switches();
        let monochrome_palette = console.uses_monochrome_palette();
        let mut screen_view = ScreenView::new();
        screen_view.set_technology(console.video_out());
        let mut this = Self {
            console: Some(console),
            platform,
            screen_view,
            running: false,
            screen_hovered: false,
            palette: PaletteChoice::default(),
            use_sgb_colors,
            persistence,
            switches,
            switch_levels: switches.iter().map(|s| s.default_high).collect(),
            monochrome_palette,
            open_panels: Vec::new(),
        };
        this.screen_view.set_persistence(persistence);
        this.refresh_palette_policy();
        this
    }

    pub fn from_debugger(
        console: Box<dyn SystemConsole>,
        screen_view: ScreenView,
        platform: Platform,
        use_sgb_colors: bool,
        persistence: bool,
    ) -> Self {
        let switches = console.console_switches();
        let monochrome_palette = console.uses_monochrome_palette();
        let mut this = Self {
            console: Some(console),
            platform,
            screen_view,
            running: false,
            screen_hovered: false,
            palette: PaletteChoice::default(),
            use_sgb_colors,
            persistence,
            switches,
            switch_levels: switches.iter().map(|s| s.default_high).collect(),
            monochrome_palette,
            open_panels: Vec::new(),
        };
        this.screen_view.set_persistence(persistence);
        this.refresh_palette_policy();
        this
    }

    /// Rebuild the renderer's colour policy from the current palette and SGB
    /// choice; a no-op for families whose frames arrive already resolved.
    fn refresh_palette_policy(&mut self) {
        let policy = gb::palette_policy(self.platform, self.palette, self.use_sgb_colors);
        self.screen_view.set_palette_policy(policy);
    }

    pub fn set_use_sgb_colors(&mut self, use_sgb: bool) {
        self.use_sgb_colors = use_sgb;
        self.refresh_palette_policy();
    }

    pub fn set_persistence(&mut self, persistence: bool) {
        self.persistence = persistence;
        self.screen_view.set_persistence(persistence);
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
    pub fn apply_frame(&mut self, display: Frame) {
        self.screen_view.apply(&display);
    }

    /// Switch to debugger mode; systems without a debugger backend come
    /// back unchanged.
    pub fn enable_debugger(self) -> Result<app::debugger::Debugger, Box<Emulator>> {
        let use_sgb_colors = self.use_sgb_colors;
        let persistence = self.persistence;
        let platform = self.platform;
        let console = self
            .console
            .expect("console present when enabling the debugger");
        app::debugger::Debugger::from_console(console, self.screen_view, platform).map_err(
            |returned| {
                let (console, screen_view) = *returned;
                Box::new(Emulator::from_debugger(
                    console,
                    screen_view,
                    platform,
                    use_sgb_colors,
                    persistence,
                ))
            },
        )
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
            Message::TogglePanel(panel) => {
                if let Some(pos) = self.open_panels.iter().position(|&p| p == panel) {
                    self.open_panels.remove(pos);
                } else {
                    self.open_panels.push(panel);
                }
            }
        }
        Task::none()
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.palette = palette;
        self.refresh_palette_policy();
    }

    pub fn view(
        &self,
        fullscreen: bool,
        play_log: &[panels::PlayLogEntry],
    ) -> Element<'_, app::Message> {
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
            return screen;
        }

        let screen_stack: Element<'_, app::Message> = if self.screen_hovered {
            use iced::Border;

            fn overlay_button_style(_theme: &iced::Theme, status: button::Status) -> button::Style {
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

        let screen_area: Element<'_, app::Message> = container(
            mouse_area(screen_stack)
                .on_enter(Message::ScreenHovered.into())
                .on_exit(Message::ScreenUnhovered.into())
                .on_move(|_| Message::ScreenHovered.into()),
        )
        .width(Fill)
        .height(Fill)
        .into();

        let ctx = panels::PanelContext {
            switches: self.switches,
            switch_levels: &self.switch_levels,
            palette: self.palette,
            use_sgb_colors: self.use_sgb_colors,
            play_log,
            has_console: !self.switches.is_empty(),
            has_display: self.monochrome_palette,
            // Capturing is always available, so the Play log is too.
            has_playlog: true,
        };

        let mut layout = row![screen_area];
        if let Some(side) = panels::side_column(&self.open_panels, &ctx) {
            layout = layout.push(side);
        }
        if let Some(rail) = panels::rail(&self.open_panels, &ctx) {
            layout = layout.push(rail);
        }
        layout.width(Fill).height(Fill).into()
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
