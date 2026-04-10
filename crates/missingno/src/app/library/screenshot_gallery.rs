use iced::{
    Alignment::Center,
    Color, Element,
    Length::Fill,
    widget::{button, column, container, image, row, scrollable, text},
};
use missingno_gb::ppu::types::palette::PaletteChoice;

use crate::app::{
    self,
    ui::{
        buttons,
        icons::{self, Icon},
        sizes::{l, m, s},
        text as app_text,
    },
    library::activity::{self, DisplayMode, FrameCapture},
};

// Catppuccin Mocha subtext0
const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

/// State for the screenshot gallery view.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct GalleryState {
    /// Session filename this gallery is showing.
    pub session_filename: String,
    /// All screenshots from the session (pre-loaded).
    pub screenshots: Vec<Screenshot>,
    /// Currently selected screenshot index.
    pub selected: usize,
    /// Which palette to render with.
    pub palette: PaletteSelection,
    /// Export scale factor.
    pub scale: u32,
}

#[derive(Clone, Debug)]
pub struct Screenshot {
    pub capture: FrameCapture,
    pub timestamp: jiff::Timestamp,
}

/// Which palette to render the screenshot with.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteSelection {
    Sgb,
    Dmg(PaletteChoice),
}

impl PaletteSelection {
    /// Derive the default palette selection from a capture's display mode.
    fn from_display_mode(mode: &DisplayMode) -> Self {
        match mode {
            DisplayMode::Sgb => Self::Sgb,
            DisplayMode::Palette(name) => {
                let choice = match name.as_str() {
                    "Green" => PaletteChoice::Green,
                    "Pocket" => PaletteChoice::Pocket,
                    "Classic" => PaletteChoice::Classic,
                    _ => PaletteChoice::default(),
                };
                Self::Dmg(choice)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectScreenshot(usize),
    SetPalette(PaletteSelection),
    SetScale(u32),
    Export,
    ExportSelected(Option<rfd::FileHandle>),
    Back,
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::ScreenshotGallery(message)
    }
}

impl GalleryState {
    /// Load gallery state from a session file.
    pub fn load(game_dir: &std::path::Path, session_filename: &str) -> Option<Self> {
        let data = activity::read_session_file(game_dir, session_filename)?;
        let screenshots: Vec<Screenshot> = data
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                activity::EventKind::Screenshot { frame } => Some(Screenshot {
                    capture: frame.clone(),
                    timestamp: e.at,
                }),
                _ => None,
            })
            .collect();

        if screenshots.is_empty() {
            return None;
        }

        let palette = PaletteSelection::from_display_mode(&screenshots[0].capture.display_mode);

        Some(Self {
            session_filename: session_filename.to_string(),
            screenshots,
            selected: 0,
            palette,
            scale: 2,
        })
    }

    /// Update selection and reset palette to the new screenshot's capture-time default.
    pub fn select(&mut self, idx: usize) {
        if idx < self.screenshots.len() {
            self.selected = idx;
            self.palette =
                PaletteSelection::from_display_mode(&self.screenshots[idx].capture.display_mode);
        }
    }

    /// Render the current selection at 1x as RGBA.
    pub fn selected_rgba(&self) -> Vec<u8> {
        let capture = &self.screenshots[self.selected].capture;
        match &self.palette {
            PaletteSelection::Sgb => capture.to_rgba_sgb_or_fallback(),
            PaletteSelection::Dmg(choice) => capture.to_rgba_with_palette_choice(*choice),
        }
    }

    /// Create a scaled image handle for the preview (nearest-neighbour).
    fn selected_image_handle_scaled(&self) -> iced::widget::image::Handle {
        let rgba = self.selected_rgba();
        let scaled = scale_nearest_neighbour(&rgba, 160, 144, self.scale);
        iced::widget::image::Handle::from_rgba(160 * self.scale, 144 * self.scale, scaled)
    }

    /// Whether the current screenshot has SGB data.
    fn has_sgb(&self) -> bool {
        self.screenshots[self.selected].capture.sgb.is_some()
    }
}

pub fn scale_nearest_neighbour(rgba: &[u8], w: u32, h: u32, scale: u32) -> Vec<u8> {
    let sw = w * scale;
    let sh = h * scale;
    let mut out = Vec::with_capacity((sw * sh * 4) as usize);
    for y in 0..sh {
        for x in 0..sw {
            let src_x = (x / scale) as usize;
            let src_y = (y / scale) as usize;
            let idx = (src_y * w as usize + src_x) * 4;
            out.extend_from_slice(&rgba[idx..idx + 4]);
        }
    }
    out
}

#[allow(private_interfaces)]
pub(crate) fn view(state: &GalleryState) -> Element<'_, app::Message> {
    let main_image = {
        let handle = state.selected_image_handle_scaled();
        let px = 160 * state.scale;
        let py = 144 * state.scale;
        container(
            image(handle)
                .width(px as f32)
                .height(py as f32)
                .content_fit(iced::ContentFit::None),
        )
        .center(Fill)
        .clip(true)
    };

    let controls = controls_panel(state);

    let top = row![main_image, controls].height(Fill);

    let thumbnail_strip = thumbnail_strip(state);

    column![top, thumbnail_strip]
        .spacing(m())
        .padding(l())
        .into()
}

fn controls_panel(state: &GalleryState) -> Element<'_, app::Message> {
    let screenshot = &state.screenshots[state.selected];
    let timestamp = activity::format_local(&screenshot.timestamp);

    let mut col = column![
        app_text::label("Screenshot"),
        app_text::detail(timestamp).color(MUTED),
    ]
    .spacing(m());

    // Palette selection
    col = col.push(app_text::label("Palette"));

    // DMG palette options
    for &choice in PaletteChoice::ALL {
        let is_selected = state.palette == PaletteSelection::Dmg(choice);
        col = col.push(palette_button(
            &format!("{choice}"),
            is_selected,
            Message::SetPalette(PaletteSelection::Dmg(choice)).into(),
        ));
    }

    // SGB option (only if the capture has SGB data)
    if state.has_sgb() {
        let is_selected = state.palette == PaletteSelection::Sgb;
        col = col.push(palette_button(
            "Super Game Boy",
            is_selected,
            Message::SetPalette(PaletteSelection::Sgb).into(),
        ));
    }

    // Scale selection
    col = col
        .push(iced::widget::Space::new().height(s()))
        .push(app_text::label("Scale"));
    let mut scale_row = row![].spacing(s());
    for &scale in &[1u32, 2, 3, 4] {
        let label = format!("{scale}x");
        let btn = if state.scale == scale {
            buttons::primary(text(label).size(13.0))
        } else {
            buttons::standard(text(label).size(13.0)).on_press(Message::SetScale(scale).into())
        };
        scale_row = scale_row.push(btn);
    }
    col = col.push(scale_row);

    // Export button
    col = col.push(iced::widget::Space::new().height(s())).push(
        buttons::primary(
            row![icons::m(Icon::Download), "Export PNG"]
                .spacing(s())
                .align_y(Center),
        )
        .on_press(Message::Export.into()),
    );

    container(scrollable(col.padding(m())).height(Fill))
        .width(250)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                ..Default::default()
            }
        })
        .into()
}

fn palette_button<'a>(
    label: &str,
    is_selected: bool,
    message: app::Message,
) -> Element<'a, app::Message> {
    let t = app_text::detail(label.to_string());
    if is_selected {
        buttons::primary(t).width(Fill).into()
    } else {
        buttons::subtle(t).on_press(message).width(Fill).into()
    }
}

fn thumbnail_strip(state: &GalleryState) -> Element<'_, app::Message> {
    let mut strip = row![].spacing(s());

    for (i, screenshot) in state.screenshots.iter().enumerate() {
        let handle = screenshot.capture.to_image_handle();
        let thumb = image(handle).width(80).height(72);
        let is_selected = i == state.selected;

        let thumb_btn = button(thumb)
            .on_press(Message::SelectScreenshot(i).into())
            .style(move |theme: &iced::Theme, status| {
                let palette = theme.extended_palette();
                let mut style = button::Style {
                    background: Some(palette.background.weak.color.into()),
                    border: iced::Border::default().rounded(4),
                    ..Default::default()
                };
                if is_selected {
                    style.border = style.border.color(palette.primary.strong.color).width(2.0);
                }
                if matches!(status, button::Status::Hovered) {
                    style.background = Some(palette.background.strong.color.into());
                }
                style
            })
            .padding(2);

        strip = strip.push(thumb_btn);
    }

    scrollable(container(strip).padding(m()))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new(),
        ))
        .into()
}
