use iced::{
    Element,
    Length::{self, Fill},
    Task,
    widget::{button, container, mouse_area, responsive, row, shader, stack, svg},
};

use missingno_session::SessionHandle;

use crate::app::system::{ConsoleSwitch, Platform, gb};
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

/// The frontend's display-presentation choices, keyed to the console's stated
/// technology by the renderer. Grid and scanlines are mutually exclusive there,
/// so both can be carried unconditionally.
#[derive(Clone, Copy)]
pub struct Presentation {
    pub use_sgb_colors: bool,
    pub persistence: bool,
    pub pixel_grid: bool,
    pub scanlines: bool,
}

/// The UI-side shell for a plain (non-debugger) game. The console lives
/// permanently in the shared session; this shell is a client — it drives the
/// session through the handle and renders the frames it publishes.
pub struct Emulator {
    /// The client handle onto the session hosting this game's console.
    handle: SessionHandle,
    /// The platform this game presents, captured at load; carried so a
    /// debugger toggle can key its panes without the console at hand.
    platform: Platform,
    screen_view: ScreenView,
    screen_hovered: bool,
    /// The user's monochrome palette choice, held so a palette change can rebuild
    /// the renderer's colour policy and the Display panel can show the selection.
    palette: PaletteChoice,
    use_sgb_colors: bool,
    persistence: bool,
    pixel_grid: bool,
    scanlines: bool,
    /// The family's latching console switches and their current levels,
    /// captured at load so the Console panel renders without reaching into the
    /// session-owned console. Empty for families with none.
    switches: &'static [ConsoleSwitch],
    switch_levels: Vec<bool>,
    /// Whether this console has a selectable monochrome palette (DMG),
    /// captured at load; gates the Display panel's palette rows.
    monochrome_palette: bool,
    /// Whether the loaded game enables Super Game Boy enhancements, captured at
    /// load; gates the SGB colour rows.
    supports_sgb: bool,
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

/// The console properties this shell caches at load, read from the console
/// once before it moves into the session (which then owns it permanently).
pub struct ConsoleFacts {
    pub switches: &'static [ConsoleSwitch],
    pub monochrome_palette: bool,
    pub supports_sgb: bool,
    pub technology: missingno_core::video::DisplayTechnology,
}

impl ConsoleFacts {
    pub fn of(console: &dyn SystemConsole) -> Self {
        Self {
            switches: console.console_switches(),
            monochrome_palette: console.uses_monochrome_palette(),
            supports_sgb: console.supports_sgb(),
            technology: console.video_out(),
        }
    }
}

impl Emulator {
    /// Build a fresh shell over the session hosting a newly loaded console.
    pub fn new(
        handle: SessionHandle,
        facts: ConsoleFacts,
        platform: Platform,
        presentation: Presentation,
    ) -> Self {
        let mut screen_view = ScreenView::new();
        screen_view.set_technology(facts.technology);
        Self::build(handle, screen_view, facts, platform, presentation)
    }

    /// Build a shell carrying a screen view across a debugger→emulator toggle.
    pub fn from_debugger(
        handle: SessionHandle,
        screen_view: ScreenView,
        facts: ConsoleFacts,
        platform: Platform,
        presentation: Presentation,
    ) -> Self {
        Self::build(handle, screen_view, facts, platform, presentation)
    }

    fn build(
        handle: SessionHandle,
        screen_view: ScreenView,
        facts: ConsoleFacts,
        platform: Platform,
        presentation: Presentation,
    ) -> Self {
        let switches = facts.switches;
        let mut this = Self {
            handle,
            platform,
            screen_view,
            screen_hovered: false,
            palette: PaletteChoice::default(),
            use_sgb_colors: presentation.use_sgb_colors,
            persistence: presentation.persistence,
            pixel_grid: presentation.pixel_grid,
            scanlines: presentation.scanlines,
            switches,
            switch_levels: switches.iter().map(|s| s.default_high).collect(),
            monochrome_palette: facts.monochrome_palette,
            supports_sgb: facts.supports_sgb,
            open_panels: Vec::new(),
        };
        this.apply_presentation();
        this.refresh_palette_policy();
        this
    }

    /// Push the frontend's presentation choices onto the renderer.
    fn apply_presentation(&mut self) {
        self.screen_view.set_persistence(self.persistence);
        self.screen_view.set_pixel_grid(self.pixel_grid);
        self.screen_view.set_scanlines(self.scanlines);
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

    pub fn set_pixel_grid(&mut self, pixel_grid: bool) {
        self.pixel_grid = pixel_grid;
        self.screen_view.set_pixel_grid(pixel_grid);
    }

    pub fn set_scanlines(&mut self, scanlines: bool) {
        self.scanlines = scanlines;
        self.screen_view.set_scanlines(scanlines);
    }

    /// The display technology the loaded console states.
    pub fn technology(&self) -> missingno_core::video::DisplayTechnology {
        self.screen_view.technology()
    }

    /// Whether the loaded game enables Super Game Boy enhancements.
    pub fn supports_sgb(&self) -> bool {
        self.supports_sgb
    }

    /// Update the displayed frame from the session's latest-frame slot.
    pub fn apply_frame(&mut self, display: Frame) {
        self.screen_view.apply(&display);
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The screen view, taken to carry across a debugger toggle.
    pub fn take_screen_view(&mut self) -> ScreenView {
        std::mem::replace(&mut self.screen_view, ScreenView::new())
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
                    // Route through the shared control path so it reaches the
                    // session-owned console.
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
            supports_sgb: self.supports_sgb,
            monochrome_palette: self.monochrome_palette,
            persistence: self.persistence,
            pixel_grid: self.pixel_grid,
            scanlines: self.scanlines,
            technology: self.technology(),
            play_log,
            has_console: !self.switches.is_empty(),
            // The Display panel now carries options for every system, so it is
            // available whenever a console is running.
            has_display: true,
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

    /// Whether the session is free-running — the session is the source of truth.
    pub fn running(&self) -> bool {
        self.handle.is_running()
    }

    pub fn run(&self) {
        self.handle.run();
    }

    pub fn pause(&self) {
        self.handle.pause();
    }

    pub fn reset(&self) {
        self.handle.reset();
    }
}
