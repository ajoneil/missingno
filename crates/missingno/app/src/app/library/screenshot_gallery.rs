use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    widget::{button, column, container, image, row, scrollable},
};
use missingno_gb::ppu::types::palette::PaletteChoice;

use crate::app::{
    self,
    library::activity::{self, DisplayMode, FrameCapture},
    ui::{
        buttons, containers,
        icons::{self, Icon},
        palette::MUTED,
        sizes::{border_s, l, m, s},
        text as app_text,
    },
};

/// State for the screenshot gallery view.
#[derive(Clone, Debug)]
pub struct GalleryState {
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
    /// CGB captures are fixed colour — no palette to choose.
    Cgb,
}

impl PaletteSelection {
    /// Derive the default palette selection from a capture's display mode.
    fn from_display_mode(mode: &DisplayMode) -> Self {
        match mode {
            DisplayMode::Sgb => Self::Sgb,
            DisplayMode::Cgb => Self::Cgb,
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

    /// The selected capture, for sizing and render-path decisions.
    pub fn selected_capture(&self) -> &FrameCapture {
        &self.screenshots[self.selected].capture
    }

    /// Render the current selection at 1x as RGBA.
    pub fn selected_rgba(&self) -> Vec<u8> {
        let capture = self.selected_capture();
        if let Some(rgba) = &capture.rgba {
            return rgba.data.clone();
        }
        match &self.palette {
            PaletteSelection::Sgb => capture.to_rgba_sgb_or_fallback(),
            PaletteSelection::Dmg(choice) => capture.to_rgba_with_palette_choice(*choice),
            PaletteSelection::Cgb => capture.to_rgba(),
        }
    }

    /// A display-correct RGBA image for PNG export at the chosen scale: integer
    /// upscale, then a horizontal stretch by the capture's pixel aspect.
    pub fn export_image(&self) -> (u32, u32, Vec<u8>) {
        let rgba = self.selected_rgba();
        let capture = self.selected_capture();
        let (w, h) = capture.dimensions();
        let out_w = ((w * self.scale) as f32 * capture.pixel_aspect())
            .round()
            .max(1.0) as u32;
        let out_h = h * self.scale;
        (
            out_w,
            out_h,
            activity::resample_nearest(&rgba, w, h, out_w, out_h),
        )
    }

    /// A display-correct preview handle plus its pixel size (nearest-neighbour).
    fn selected_scaled(&self) -> (iced::widget::image::Handle, u32, u32) {
        let (width, height, scaled) = self.export_image();
        (
            iced::widget::image::Handle::from_rgba(width, height, scaled),
            width,
            height,
        )
    }

    /// Whether the current screenshot has SGB data.
    fn has_sgb(&self) -> bool {
        self.screenshots[self.selected].capture.sgb.is_some()
    }
}

#[allow(private_interfaces)]
pub(crate) fn view(state: &GalleryState) -> Element<'_, app::Message> {
    let main_image = {
        let (handle, px, py) = state.selected_scaled();
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

    // Palette selection applies only to GB shade captures; CGB and
    // self-sized RGBA captures are fixed colour.
    if !matches!(screenshot.capture.display_mode, DisplayMode::Cgb)
        && screenshot.capture.rgba.is_none()
    {
        col = col.push(app_text::label("Palette"));

        let mut palette_col = column![].spacing(2);
        for &choice in PaletteChoice::ALL {
            let is_selected = state.palette == PaletteSelection::Dmg(choice);
            palette_col = palette_col.push(palette_button(
                &format!("{choice}"),
                is_selected,
                Message::SetPalette(PaletteSelection::Dmg(choice)).into(),
            ));
        }
        if state.has_sgb() {
            let is_selected = state.palette == PaletteSelection::Sgb;
            palette_col = palette_col.push(palette_button(
                "Super Game Boy",
                is_selected,
                Message::SetPalette(PaletteSelection::Sgb).into(),
            ));
        }
        col = col.push(palette_col);
    }

    // Scale selection
    col = col
        .push(iced::widget::Space::new().height(s()))
        .push(app_text::label("Scale"));
    let mut scale_row = row![].spacing(s());
    for &scale in &[1u32, 2, 3, 4] {
        let label = format!("{scale}x");
        let btn = if state.scale == scale {
            buttons::selected(app_text::detail(label))
        } else {
            buttons::subtle(app_text::detail(label)).on_press(Message::SetScale(scale).into())
        };
        scale_row = scale_row.push(btn);
    }
    col = col.push(scale_row);

    // Export button
    col = col
        .push(iced::widget::Space::new().height(s()))
        .push(app::automation::tag(
            app::automation::ids::GALLERY_EXPORT,
            buttons::primary(
                row![icons::m(Icon::Save), "Export PNG"]
                    .spacing(s())
                    .align_y(Center),
            )
            .on_press(Message::Export.into()),
        ));

    container(scrollable(col.padding(m())).height(Fill))
        .width(250)
        .style(containers::sidebar)
        .into()
}

fn palette_button<'a>(
    label: &str,
    is_selected: bool,
    message: app::Message,
) -> Element<'a, app::Message> {
    let t = app_text::detail(label.to_string());
    if is_selected {
        buttons::selected(t).width(Fill).into()
    } else {
        buttons::subtle(t).on_press(message).width(Fill).into()
    }
}

fn thumbnail_strip(state: &GalleryState) -> Element<'_, app::Message> {
    let mut strip = row![].spacing(s());

    for (i, screenshot) in state.screenshots.iter().enumerate() {
        let handle = screenshot.capture.to_image_handle();
        let thumb_height = 72u32;
        let thumb_width = (thumb_height as f32 * screenshot.capture.display_aspect())
            .round()
            .max(1.0) as u32;
        let thumb = image(handle)
            .width(thumb_width as f32)
            .height(thumb_height as f32)
            .content_fit(iced::ContentFit::Fill);
        let is_selected = i == state.selected;

        let thumb_btn = button(thumb)
            .on_press(Message::SelectScreenshot(i).into())
            .style(move |theme: &iced::Theme, status| {
                let palette = theme.extended_palette();
                let mut style = button::Style {
                    background: Some(palette.background.weak.color.into()),
                    border: iced::Border::default().rounded(border_s()),
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
