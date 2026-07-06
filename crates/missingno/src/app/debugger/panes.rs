use core::fmt;
use std::collections::{BTreeSet, HashMap};

use iced::{
    Border, Color, Element, Theme,
    widget::{
        container, pane_grid,
        pane_grid::Axis::{Horizontal, Vertical},
        toggler,
    },
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        self,
        audio::AudioPane,
        inspect::InspectSource,
        instructions::InstructionsPane,
        layout,
        ppu::{
            sprites::{self, SpritesPane},
            tile_maps::TileMapPane,
            tiles::{self, TilesPane},
        },
        screen::{self, ScreenPane},
    },
    screen::ScreenView,
    ui::{
        fonts,
        icons::Icon,
        palette,
        sizes::{self as sizes, s, xs},
    },
};
use missingno_gb::debugger::cdl::CdlWindow;
use missingno_gb::debugger::symbols::SymbolTable;
use missingno_gb::ppu::types::{
    palette::{Palette, PaletteChoice},
    tiles::TileMapId,
};

// Frame-carrying messages are produced once per frame; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
    ClosePane(DebuggerPane),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    Pane(PaneMessage),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PaneMessage {
    Screen(screen::Message),
    Sprites(sprites::Message),
    Tiles(tiles::Message),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Debugger(debugger::Message::Pane(message))
    }
}

/// What a pane renders from: the live console while paused, or the per-vblank
/// snapshot while the core runs. `None` means the core is away and no snapshot
/// has arrived yet.
#[derive(Clone, Copy)]
pub struct PaneContext<'b> {
    /// The Game Boy family's inspection surface, when that family is live.
    pub gb: Option<&'b dyn InspectSource>,
    #[cfg(feature = "vcs")]
    pub vcs: Option<&'b crate::app::debugger::vcs::VcsInspectState>,
    #[cfg(feature = "sms")]
    pub sms: Option<&'b crate::app::debugger::sms::SmsInspectState>,
    pub breakpoints: &'b BTreeSet<u16>,
    pub colors: &'b ConsoleColors,
    pub symbols: &'b SymbolTable,
    pub cdl: &'b CdlWindow,
}

/// One debugger pane behind the grid. Implementations live with their pane
/// modules; adding a pane means implementing this and adding one
/// [`PANE_REGISTRY`] entry.
pub trait Pane {
    fn kind(&self) -> DebuggerPane;
    fn view<'a>(&'a self, ctx: Option<&PaneContext<'_>>) -> pane_grid::Content<'a, app::Message>;
    fn on_message(&mut self, _message: &PaneMessage) {}
    fn set_palette(&mut self, _palette: PaletteChoice) {}
    fn set_frame_blending(&mut self, _blend: bool) {}
    /// The live screen state, so it can carry across a debugger↔emulator
    /// toggle. Only the screen pane has one.
    fn screen_view(&self) -> Option<ScreenView> {
        None
    }
    fn adopt_screen_view(&mut self, _view: ScreenView) {}
}

pub struct PaneDescriptor {
    pub kind: DebuggerPane,
    pub icon: Icon,
    /// Stable display name; also the key saved layouts refer to panes by.
    pub label: &'static str,
    pub(super) construct: fn() -> Box<dyn Pane>,
}

/// Everything a family brings to the pane grid: its pane set, the key its
/// layout persists under, and its out-of-the-box arrangement.
pub struct Family {
    pub registry: &'static [PaneDescriptor],
    pub layout_key: &'static str,
    default_layout: fn() -> Option<pane_grid::State<Box<dyn Pane>>>,
}

pub static GB_FAMILY: Family = Family {
    registry: PANE_REGISTRY,
    // The empty key keeps the Game Boy's pre-family layout filename.
    layout_key: "",
    default_layout: gb_default_layout,
};

#[cfg(feature = "vcs")]
pub static VCS_FAMILY: Family = Family {
    registry: VCS_PANE_REGISTRY,
    layout_key: "vcs",
    default_layout: vcs_default_layout,
};

/// Every registered pane across all families, for label and kind lookups.
pub fn all_descriptors() -> impl Iterator<Item = &'static PaneDescriptor> {
    #[cfg(feature = "vcs")]
    let vcs = VCS_PANE_REGISTRY.iter();
    #[cfg(not(feature = "vcs"))]
    let vcs = [].iter();
    #[cfg(feature = "sms")]
    let sms = SMS_PANE_REGISTRY.iter();
    #[cfg(not(feature = "sms"))]
    let sms = [].iter();
    PANE_REGISTRY.iter().chain(vcs).chain(sms)
}

#[cfg(feature = "sms")]
pub static SMS_FAMILY: Family = Family {
    registry: SMS_PANE_REGISTRY,
    layout_key: "sms",
    default_layout: sms_default_layout,
};

#[cfg(feature = "sms")]
pub static SMS_PANE_REGISTRY: &[PaneDescriptor] = &[
    PaneDescriptor {
        kind: DebuggerPane::Screen,
        icon: Icon::Monitor,
        label: "Screen",
        construct: || Box::new(ScreenPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::SmsCpu,
        icon: Icon::FileText,
        label: "Z80",
        construct: || Box::new(crate::app::debugger::sms::CpuPane),
    },
    PaneDescriptor {
        kind: DebuggerPane::SmsVdp,
        icon: Icon::Image,
        label: "VDP",
        construct: || Box::new(crate::app::debugger::sms::VdpPane),
    },
];

#[cfg(feature = "vcs")]
pub static VCS_PANE_REGISTRY: &[PaneDescriptor] = &[
    PaneDescriptor {
        kind: DebuggerPane::Screen,
        icon: Icon::Monitor,
        label: "Screen",
        construct: || Box::new(ScreenPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::VcsCpu,
        icon: Icon::FileText,
        label: "6507",
        construct: || Box::new(crate::app::debugger::vcs::CpuPane),
    },
    PaneDescriptor {
        kind: DebuggerPane::VcsTia,
        icon: Icon::Sliders,
        label: "TIA",
        construct: || Box::new(crate::app::debugger::vcs::TiaPane),
    },
];

pub static PANE_REGISTRY: &[PaneDescriptor] = &[
    PaneDescriptor {
        kind: DebuggerPane::Screen,
        icon: Icon::Monitor,
        label: "Screen",
        construct: || Box::new(ScreenPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::Instructions,
        icon: Icon::FileText,
        label: "Instructions",
        construct: || Box::new(InstructionsPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::Tiles,
        icon: Icon::Grid,
        label: "Tiles",
        construct: || Box::new(TilesPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::TileMap(TileMapId(0)),
        icon: Icon::Image,
        label: "Tile Map 0",
        construct: || Box::new(TileMapPane::new(TileMapId(0))),
    },
    PaneDescriptor {
        kind: DebuggerPane::TileMap(TileMapId(1)),
        icon: Icon::Image,
        label: "Tile Map 1",
        construct: || Box::new(TileMapPane::new(TileMapId(1))),
    },
    PaneDescriptor {
        kind: DebuggerPane::Sprites,
        icon: Icon::Human,
        label: "Sprites",
        construct: || Box::new(SpritesPane::new()),
    },
    PaneDescriptor {
        kind: DebuggerPane::Audio,
        icon: Icon::Sliders,
        label: "Audio",
        construct: || Box::new(AudioPane::new()),
    },
];

pub struct DebuggerPanes {
    family: &'static Family,
    panes: Option<pane_grid::State<Box<dyn Pane>>>,
    handles: HashMap<DebuggerPane, pane_grid::Pane>,
    palette: PaletteChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerPane {
    Screen,
    Instructions,
    Tiles,
    TileMap(TileMapId),
    Sprites,
    Audio,
    #[cfg(feature = "vcs")]
    VcsCpu,
    #[cfg(feature = "vcs")]
    VcsTia,
    #[cfg(feature = "sms")]
    SmsCpu,
    #[cfg(feature = "sms")]
    SmsVdp,
}

impl DebuggerPane {
    pub fn descriptor(&self) -> &'static PaneDescriptor {
        all_descriptors()
            .find(|descriptor| descriptor.kind == *self)
            .expect("every pane kind is registered")
    }

    pub fn icon(&self) -> Icon {
        self.descriptor().icon
    }

    fn construct(&self) -> Box<dyn Pane> {
        (self.descriptor().construct)()
    }
}

impl fmt::Display for DebuggerPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.descriptor().label)
    }
}

impl DebuggerPanes {
    pub fn new(family: &'static Family) -> Self {
        Self::build(family, None)
    }

    pub fn with_screen(family: &'static Family, screen_view: ScreenView) -> Self {
        Self::build(family, Some(screen_view))
    }

    fn build(family: &'static Family, screen_view: Option<ScreenView>) -> Self {
        let panes = layout::load(family.layout_key)
            .and_then(|saved| saved.into_state())
            .unwrap_or_else(family.default_layout);

        let mut this = Self {
            family,
            handles: panes
                .as_ref()
                .map(|state| {
                    state
                        .iter()
                        .map(|(&handle, pane)| (pane.kind(), handle))
                        .collect()
                })
                .unwrap_or_default(),
            panes,
            palette: PaletteChoice::default(),
        };
        if let Some(view) = screen_view {
            this.adopt_screen_view(view);
        }
        this
    }
}

/// The Game Boy's out-of-the-box arrangement: instructions beside the screen.
fn gb_default_layout() -> Option<pane_grid::State<Box<dyn Pane>>> {
    let (mut panes, instructions_handle) =
        pane_grid::State::new(DebuggerPane::Instructions.construct());
    let (_, split) = panes
        .split(
            Vertical,
            instructions_handle,
            DebuggerPane::Screen.construct(),
        )
        .unwrap();
    panes.resize(split, 1.0 / 3.0);
    Some(panes)
}

/// The SMS starts with the Z80 beside the screen, the VDP below.
#[cfg(feature = "sms")]
fn sms_default_layout() -> Option<pane_grid::State<Box<dyn Pane>>> {
    let (mut panes, cpu_handle) = pane_grid::State::new(DebuggerPane::SmsCpu.construct());
    let (screen_handle, split) = panes
        .split(Vertical, cpu_handle, DebuggerPane::Screen.construct())
        .unwrap();
    panes.resize(split, 1.0 / 3.0);
    panes
        .split(Horizontal, screen_handle, DebuggerPane::SmsVdp.construct())
        .unwrap();
    Some(panes)
}

/// The VCS starts with the 6507 beside the screen, the TIA below.
#[cfg(feature = "vcs")]
fn vcs_default_layout() -> Option<pane_grid::State<Box<dyn Pane>>> {
    let (mut panes, cpu_handle) = pane_grid::State::new(DebuggerPane::VcsCpu.construct());
    let (screen_handle, split) = panes
        .split(Vertical, cpu_handle, DebuggerPane::Screen.construct())
        .unwrap();
    panes.resize(split, 1.0 / 3.0);
    panes
        .split(Horizontal, screen_handle, DebuggerPane::VcsTia.construct())
        .unwrap();
    Some(panes)
}

impl DebuggerPanes {
    fn adopt_screen_view(&mut self, view: ScreenView) {
        if let Some(panes) = &mut self.panes {
            for (_, pane) in panes.iter_mut() {
                if pane.kind() == DebuggerPane::Screen {
                    pane.adopt_screen_view(view);
                    return;
                }
            }
        }
    }

    pub fn take_screen_view(&self) -> ScreenView {
        if let Some(panes) = &self.panes {
            for (_, pane) in panes.iter() {
                if let Some(view) = pane.screen_view() {
                    return view;
                }
            }
        }
        ScreenView::new()
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ShowPane(pane) => {
                if !self.handles.contains_key(&pane) {
                    let instance = pane.construct();

                    if let Some(panes) = &mut self.panes {
                        let (last_pane, _) = panes.iter().last().unwrap();
                        let (handle, _) = panes.split(Horizontal, *last_pane, instance).unwrap();
                        self.handles.insert(pane, handle);
                    } else {
                        let (panes, handle) = pane_grid::State::new(instance);
                        self.handles.insert(pane, handle);
                        self.panes = Some(panes);
                    }
                    self.persist();
                }
            }
            Message::ClosePane(pane) => {
                if let Some(&handle) = self.handles.get(&pane) {
                    if self.handles.len() == 1 {
                        self.panes = None;
                        self.handles.clear();
                    } else if let Some(panes) = &mut self.panes {
                        panes.close(handle);
                        self.handles.remove(&pane);
                    }
                    self.persist();
                }
            }

            Message::ResizePane(resize) => {
                if let Some(panes) = &mut self.panes {
                    panes.resize(resize.split, resize.ratio);
                }
            }
            Message::DragPane(drag) => {
                if let pane_grid::DragEvent::Dropped { pane, target } = drag {
                    if let Some(panes) = &mut self.panes {
                        panes.drop(pane, target);
                    }
                    self.persist();
                }
            }

            Message::Pane(pane_message) => {
                if let Some(panes) = &mut self.panes {
                    panes
                        .iter_mut()
                        .for_each(|(_, pane)| pane.on_message(&pane_message));
                }
            }
        }
    }

    fn persist(&self) {
        layout::save(self.family.layout_key, self.panes.as_ref());
    }

    pub fn palette(&self) -> &Palette {
        self.palette.palette()
    }

    pub fn set_frame_blending(&mut self, blend: bool) {
        if let Some(panes) = &mut self.panes {
            panes
                .iter_mut()
                .for_each(|(_, pane)| pane.set_frame_blending(blend));
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.palette = palette;
        if let Some(panes) = &mut self.panes {
            panes
                .iter_mut()
                .for_each(|(_, pane)| pane.set_palette(palette));
        }
    }

    /// The pane grid, rendered from the live console while paused or the
    /// per-vblank snapshot while the core runs on the emu thread. Without a
    /// context (core away, no snapshot yet) panes show placeholders — except
    /// the screen, which always renders its own live frame. While running,
    /// breakpoint-gutter clicks in the instructions pane still emit their
    /// messages, but the run doesn't stop until the core does.
    pub fn view<'a>(&'a self, ctx: Option<PaneContext<'_>>) -> Element<'a, app::Message> {
        if let Some(panes) = &self.panes {
            pane_grid(panes, move |_handle, instance, _is_maximized| {
                instance.view(ctx.as_ref())
            })
            .on_resize(10.0, |resize| Message::ResizePane(resize).into())
            .on_drag(|drag| Message::DragPane(drag).into())
            .spacing(s())
            .into()
        } else {
            iced::widget::Space::new()
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        }
    }

    pub fn plane_shown(&self, plane: DebuggerPane) -> bool {
        self.handles.contains_key(&plane)
    }

    pub fn available_panes(&self) -> impl Iterator<Item = DebuggerPane> {
        self.family
            .registry
            .iter()
            .map(|descriptor| descriptor.kind)
    }
}

/// Ratios only change through resize drags, which have no end event to hook a
/// save to — persist the final layout when the debugger goes away instead.
impl Drop for DebuggerPanes {
    fn drop(&mut self) {
        self.persist();
    }
}

impl Message {
    pub fn if_shown(pane: DebuggerPane, shown: bool) -> Self {
        if shown {
            Message::ClosePane(pane)
        } else {
            Message::ShowPane(pane)
        }
    }
}

pub fn running_placeholder(label: &str) -> pane_grid::Content<'_, app::Message> {
    pane(
        title_bar(label),
        container(iced::widget::text("Running…").color(palette::MUTED))
            .center(iced::Length::Fill)
            .into(),
    )
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, app::Message>,
    content: Element<'a, app::Message>,
) -> pane_grid::Content<'a, app::Message> {
    pane_grid::Content::new(container(content).padding([2.0, 2.0]).clip(true))
        .title_bar(title)
        .style(pane_style)
}

pub fn pane_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.base.color.into()),
        border: Border::default()
            .rounded(sizes::border_s())
            .width(1.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
        ..Default::default()
    }
}

fn title_style(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette::MUTED),
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.03).into()),
        ..Default::default()
    }
}

pub fn title_bar(label: &str) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(
        container(iced::widget::text(label).font(fonts::title()).size(13.0)).padding([xs(), s()]),
    )
    .style(title_style)
}

pub fn title_bar_with_detail<'a>(
    label: &'a str,
    detail: impl Into<Element<'a, app::Message>>,
) -> pane_grid::TitleBar<'a, app::Message> {
    build_title_bar(
        iced::widget::text(label)
            .font(fonts::title())
            .size(13.0)
            .into(),
        detail,
    )
}

fn build_title_bar<'a>(
    title: Element<'a, app::Message>,
    detail: impl Into<Element<'a, app::Message>>,
) -> pane_grid::TitleBar<'a, app::Message> {
    pane_grid::TitleBar::new(container(title).padding([xs(), s()]))
        // +1px top padding nudge: the detail font (monospace 11px) is shorter
        // than the title font (Chakra Petch 13px), so it needs a small offset
        // to visually center within the title bar height.
        .controls(Element::from(container(detail).padding([xs() + 1.0, s()])))
        .always_show_controls()
        .style(title_style)
}

pub fn checkbox_title_bar(label: &str, checked: bool) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(
        container(
            toggler(checked)
                .label(label)
                .font(fonts::title())
                .size(13.0),
        )
        .padding([xs(), s()]),
    )
    .style(title_style)
}
