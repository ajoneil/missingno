use core::fmt;
use std::collections::BTreeSet;

use iced::{
    Border, Color, Element, Theme,
    widget::{
        button, container, pane_grid,
        pane_grid::Axis::{Horizontal, Vertical},
        pick_list,
    },
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        self,
        audio_scope::AudioScopePane,
        disassembly::{self, DisasmPaneData, DisassemblyPane},
        graphics::{
            atlas::{self, TileAtlasPane},
            map::{self, TileMapPane},
            objects::{self, ObjectTablePane},
        },
        layout,
        memory::{self, MemoryPane, MemoryPaneData, MemorySelection},
        screen::{self, ScreenPane},
    },
    system::{Platform, gb},
    ui::{
        fonts,
        icons::Icon,
        palette,
        sizes::{self as sizes, s, xs},
    },
};
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::Watch;
use missingno_core::video::DisplayTechnology;
use missingno_core::waveform::ChannelWave;
use missingno_gb::ppu::types::{
    palette::{Palette, PaletteChoice},
    tiles::TileMapId,
};
use missingno_iced::{PalettePolicy, ScreenView};

// Frame-carrying messages are produced once per frame; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    /// The icon-rail button for a pane kind. A single-instance kind toggles; an
    /// instanceable kind opens a fresh instance.
    RailClick(DebuggerPane),
    /// The title-bar close control of one open pane, identified by its handle.
    CloseHandle(pane_grid::Pane),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    /// A pane message delivered to every pane; single-instance panes and cache
    /// refreshes ride this.
    Broadcast(PaneMessage),
    /// A pane message delivered to one instance, so a dropdown change in one
    /// instanceable pane leaves its siblings untouched.
    Targeted(pane_grid::Pane, PaneMessage),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PaneMessage {
    Screen(screen::Message),
    Sprites(objects::Message),
    Tiles(atlas::Message),
    TileMap(map::Message),
    Memory(memory::Message),
    Disassembly(disassembly::Message),
}

impl PaneMessage {
    /// Deliver this message to just the pane behind `handle`.
    pub fn to(self, handle: pane_grid::Pane) -> app::Message {
        Message::Targeted(handle, self).into()
    }
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
    /// The Game Boy family's render palettes, when that family is live; the
    /// graphics panes colour their decoded indices through it.
    pub colors: Option<&'b ConsoleColors>,
    pub breakpoints: &'b BTreeSet<u32>,
    /// The active watches, so a disassembly row can mark itself when it composes
    /// one (a switchable-bank gutter watch).
    pub watches: &'b [Watch],
    /// The memory viewer's visible bytes for its current selection, copied at
    /// context-build time; `None` when no memory pane is shown.
    pub memory: Option<MemoryPaneData<'b>>,
    /// The disassembly rows built for the current PC; `None` when no
    /// disassembly pane is shown or the running snapshot can't fuel the walk.
    pub disasm: Option<DisasmPaneData<'b>>,
    /// The per-channel waveform windows for the audio scope; `None` when the
    /// family captures none or capture is off. Held oldest-first, owned by the
    /// view that built the context.
    pub waves: Option<&'b [ChannelWave]>,
    /// The decoded graphics surfaces for the tile/map/object panes; `None` when
    /// the family exposes none or graphics capture is off. Owned by the view
    /// that built the context.
    pub graphics: Option<&'b GraphicsView>,
}

/// One debugger pane behind the grid. Implementations live with their pane
/// modules; adding a pane means implementing this and adding one
/// [`PANE_REGISTRY`] entry.
pub trait Pane {
    fn kind(&self) -> DebuggerPane;
    /// Render the pane. `id` is its grid handle, threaded so the shared title
    /// bar can build the close control and instanceable panes can target their
    /// own dropdown messages.
    fn view<'a>(
        &'a self,
        ctx: Option<&PaneContext<'_>>,
        id: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message>;
    fn on_message(&mut self, _message: &PaneMessage) {}
    /// The source this instanceable pane currently shows (atlas index, map id,
    /// or memory region), so a new sibling can default to the first unshown
    /// one. `None` for a single-instance pane.
    fn source_index(&self) -> Option<usize> {
        None
    }
    /// Point a freshly-opened instanceable pane at a source (its dropdown
    /// default, or a restored layout selection).
    fn set_source_index(&mut self, _index: usize) {}
    /// Restore the secondary scroll offset a pane persists — only the memory
    /// viewer has one, applied after its region is set.
    fn set_source_offset(&mut self, _offset: u32) {}
    /// The scroll offset the memory viewer persists into a saved layout.
    fn source_offset(&self) -> Option<u32> {
        None
    }
    /// The memory viewer's current region/offset selection, so the context
    /// builder can copy the right bytes. Only the memory pane has one.
    fn memory_selection(&self) -> Option<MemorySelection> {
        None
    }
    /// The disassembly pane's user-set walk anchor, so the context builder walks
    /// from there instead of the PC. Only the disassembly pane has one.
    fn disasm_anchor(&self) -> Option<u32> {
        None
    }
    /// Point the screen pane at the technology the core states, so a freshly
    /// opened screen renders at the right aspect and persistence.
    fn set_technology(&mut self, _technology: DisplayTechnology) {}
    /// Install the app's colour policy on the screen pane's renderer.
    fn set_palette_policy(&mut self, _policy: Option<Box<dyn PalettePolicy>>) {}
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
    /// Whether the rail opens a fresh instance each click (many can coexist,
    /// each owning its source selection) rather than toggling a single pane.
    pub instanceable: bool,
    pub(super) construct: fn() -> Box<dyn Pane>,
}

/// Everything a family brings to the pane grid: the platforms it serves, its
/// pane set, the key its layout persists under, and its out-of-the-box
/// arrangement.
pub struct Family {
    pub platforms: &'static [Platform],
    pub registry: &'static [PaneDescriptor],
    pub layout_key: &'static str,
    default_layout: fn() -> Option<pane_grid::State<Box<dyn Pane>>>,
}

pub static GB_FAMILY: Family = Family {
    platforms: &[Platform::GameBoy, Platform::GameBoyColor],
    registry: PANE_REGISTRY,
    // The empty key keeps the Game Boy's pre-family layout filename.
    layout_key: "",
    default_layout: gb_default_layout,
};

pub static VCS_FAMILY: Family = Family {
    platforms: &[Platform::AtariVcs],
    registry: VCS_PANE_REGISTRY,
    layout_key: "vcs",
    default_layout: disassembly_screen_memory_layout,
};

/// The panes a platform presents. `None` only for a platform this build left
/// out, which cannot reach here — the same feature gates its debugger.
pub fn family_for(platform: Platform) -> Option<&'static Family> {
    PANE_FAMILIES
        .iter()
        .copied()
        .find(|family| family.platforms.contains(&platform))
}

/// Every family's pane set, for label and kind lookups across saved layouts.
static PANE_FAMILIES: &[&Family] = &[
    &GB_FAMILY,
    &VCS_FAMILY,
    #[cfg(feature = "sms")]
    &SMS_FAMILY,
    &SG1000_FAMILY,
    #[cfg(feature = "nes")]
    &NES_FAMILY,
];

/// Every registered pane across all families, for label and kind lookups.
pub fn all_descriptors() -> impl Iterator<Item = &'static PaneDescriptor> {
    PANE_FAMILIES
        .iter()
        .flat_map(|family| family.registry.iter())
}

#[cfg(feature = "nes")]
pub static NES_FAMILY: Family = Family {
    platforms: &[Platform::Nes],
    registry: NES_PANE_REGISTRY,
    layout_key: "nes",
    default_layout: disassembly_screen_memory_layout,
};

/// The NES presents the screen and the two generic code/data panes; its 2A03
/// and 2C02 register state lives in the sidebar.
#[cfg(feature = "nes")]
pub static NES_PANE_REGISTRY: &[PaneDescriptor] =
    &[SCREEN_DESCRIPTOR, MEMORY_DESCRIPTOR, DISASSEMBLY_DESCRIPTOR];

#[cfg(feature = "sms")]
pub static SMS_FAMILY: Family = Family {
    platforms: &[Platform::MasterSystem],
    registry: SMS_PANE_REGISTRY,
    layout_key: "sms",
    default_layout: disassembly_screen_memory_layout,
};

/// The SMS presents the screen and the two generic code/data panes; its Z80,
/// VDP, mapper, and PSG state lives in the sidebar.
#[cfg(feature = "sms")]
pub static SMS_PANE_REGISTRY: &[PaneDescriptor] =
    &[SCREEN_DESCRIPTOR, MEMORY_DESCRIPTOR, DISASSEMBLY_DESCRIPTOR];

pub static SG1000_FAMILY: Family = Family {
    platforms: &[Platform::Sg1000],
    registry: SG1000_PANE_REGISTRY,
    layout_key: "sg1000",
    default_layout: disassembly_screen_memory_layout,
};

/// The SG-1000 presents the screen, the two generic code/data panes, the VDP's
/// three graphics surfaces and the audio scope over its PSG; its Z80, VDP, and
/// PSG register state lives in the sidebar.
pub static SG1000_PANE_REGISTRY: &[PaneDescriptor] = &[
    SCREEN_DESCRIPTOR,
    MEMORY_DESCRIPTOR,
    DISASSEMBLY_DESCRIPTOR,
    TILES_DESCRIPTOR,
    TILE_MAP_DESCRIPTOR,
    SPRITES_DESCRIPTOR,
    AUDIO_DESCRIPTOR,
];

pub static VCS_PANE_REGISTRY: &[PaneDescriptor] = &[
    SCREEN_DESCRIPTOR,
    MEMORY_DESCRIPTOR,
    DISASSEMBLY_DESCRIPTOR,
    AUDIO_DESCRIPTOR,
];

/// The console's own picture, registered for every family.
const SCREEN_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Screen,
    icon: Icon::Monitor,
    label: "Screen",
    instanceable: false,
    construct: || Box::new(ScreenPane::new()),
};

/// The generic memory viewer, registered for every family — it reads the
/// seam's named regions and side-effect-free peeks while paused.
const MEMORY_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Memory,
    icon: Icon::MemoryStick,
    label: "Memory",
    instanceable: true,
    construct: || Box::new(MemoryPane::new()),
};

/// The generic disassembly, registered for every family — it walks and decodes
/// through the seam's instruction set, or shows raw bytes where there is none.
const DISASSEMBLY_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Disassembly,
    icon: Icon::Terminal,
    label: "Disassembly",
    instanceable: false,
    construct: || Box::new(DisassemblyPane::new()),
};

pub static PANE_REGISTRY: &[PaneDescriptor] = &[
    SCREEN_DESCRIPTOR,
    MEMORY_DESCRIPTOR,
    DISASSEMBLY_DESCRIPTOR,
    TILES_DESCRIPTOR,
    TILE_MAP_DESCRIPTOR,
    SPRITES_DESCRIPTOR,
    AUDIO_DESCRIPTOR,
];

/// The three graphics surfaces, registered for every family whose core decodes
/// its video memory into the seam's [`GraphicsView`].
const TILES_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Tiles,
    icon: Icon::Grid,
    label: "Tiles",
    instanceable: true,
    construct: || Box::new(TileAtlasPane::new()),
};

const TILE_MAP_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::TileMap,
    icon: Icon::Image,
    label: "Tile Map",
    instanceable: true,
    construct: || Box::new(TileMapPane::new(TileMapId(0))),
};

const SPRITES_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Sprites,
    icon: Icon::Human,
    label: "Sprites",
    instanceable: false,
    construct: || Box::new(ObjectTablePane::new()),
};

/// The generic audio scope: registered for every family whose core captures
/// per-channel waveforms. Reads the seam's [`ChannelWave`]s while capture is on.
const AUDIO_DESCRIPTOR: PaneDescriptor = PaneDescriptor {
    kind: DebuggerPane::Audio,
    icon: Icon::Speaker,
    label: "Audio",
    instanceable: false,
    construct: || Box::new(AudioScopePane::new()),
};

pub struct DebuggerPanes {
    family: &'static Family,
    panes: Option<pane_grid::State<Box<dyn Pane>>>,
    palette: PaletteChoice,
    /// The technology the core states, applied to any screen pane the grid
    /// builds — including one reopened from the rail after being closed.
    screen_technology: DisplayTechnology,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerPane {
    Screen,
    Memory,
    Disassembly,
    Tiles,
    TileMap,
    Sprites,
    Audio,
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

    pub fn instanceable(&self) -> bool {
        self.descriptor().instanceable
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
    pub fn with_screen(family: &'static Family, screen_view: ScreenView) -> Self {
        let panes = layout::load(family.layout_key)
            .and_then(|saved| saved.into_state())
            .unwrap_or_else(family.default_layout);

        let mut this = Self {
            family,
            panes,
            palette: PaletteChoice::default(),
            screen_technology: screen_view.technology(),
        };
        this.with_screen_pane(|pane| pane.adopt_screen_view(screen_view));
        this
    }

    pub fn screen_technology(&self) -> DisplayTechnology {
        self.screen_technology
    }
}

/// The Game Boy's out-of-the-box arrangement: disassembly beside the screen.
fn gb_default_layout() -> Option<pane_grid::State<Box<dyn Pane>>> {
    let (mut panes, disassembly_handle) =
        pane_grid::State::new(DebuggerPane::Disassembly.construct());
    let (_, split) = panes
        .split(
            Vertical,
            disassembly_handle,
            DebuggerPane::Screen.construct(),
        )
        .unwrap();
    panes.resize(split, 1.0 / 3.0);
    Some(panes)
}

/// The shared default for a register-dump family — the NES, SG-1000, SMS and
/// VCS, whose chip state lives in the sidebar: disassembly beside the screen,
/// memory below.
fn disassembly_screen_memory_layout() -> Option<pane_grid::State<Box<dyn Pane>>> {
    let (mut panes, disassembly_handle) =
        pane_grid::State::new(DebuggerPane::Disassembly.construct());
    let (screen_handle, split) = panes
        .split(
            Vertical,
            disassembly_handle,
            DebuggerPane::Screen.construct(),
        )
        .unwrap();
    panes.resize(split, 1.0 / 3.0);
    panes
        .split(Horizontal, screen_handle, DebuggerPane::Memory.construct())
        .unwrap();
    Some(panes)
}

impl DebuggerPanes {
    /// Reach the open screen pane, if the grid holds one; the first in grid
    /// order, since the kind is single-instance.
    fn with_screen_pane(&mut self, apply: impl FnOnce(&mut dyn Pane)) {
        if let Some(panes) = &mut self.panes
            && let Some((_, pane)) = panes
                .iter_mut()
                .find(|(_, pane)| pane.kind() == DebuggerPane::Screen)
        {
            apply(pane.as_mut());
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
            Message::RailClick(kind) => {
                // A single-instance kind toggles; an already-open one closes.
                // An instanceable kind always opens a fresh instance.
                if !kind.instanceable()
                    && let Some(handle) = self.handle_of_kind(kind)
                {
                    self.close_handle(handle);
                } else {
                    self.open_instance(kind);
                }
            }
            Message::CloseHandle(handle) => self.close_handle(handle),

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

            Message::Broadcast(pane_message) => {
                if let Some(panes) = &mut self.panes {
                    panes
                        .iter_mut()
                        .for_each(|(_, pane)| pane.on_message(&pane_message));
                }
                // The screen pane's device/raw toggle rides a broadcast and
                // changes per-pane state the layout persists.
                if matches!(
                    pane_message,
                    PaneMessage::Screen(screen::Message::ToggleDeviceSimulation)
                ) {
                    self.persist();
                }
            }
            Message::Targeted(handle, pane_message) => {
                if let Some(panes) = &mut self.panes
                    && let Some(pane) = panes.get_mut(handle)
                {
                    pane.on_message(&pane_message);
                }
            }
        }
    }

    /// Open a new instance of `kind`, defaulting an instanceable pane's source
    /// to the first one its siblings don't already show, and place it with the
    /// existing insertion logic.
    fn open_instance(&mut self, kind: DebuggerPane) {
        let mut pane = kind.construct();
        if kind.instanceable() {
            let default = first_unshown_source(self.source_indices(kind));
            pane.set_source_index(default);
        }
        if kind == DebuggerPane::Screen {
            pane.set_technology(self.screen_technology);
            pane.set_palette_policy(self.screen_palette_policy());
        }

        if let Some(panes) = &mut self.panes {
            let (last_pane, _) = panes.iter().last().unwrap();
            panes.split(Horizontal, *last_pane, pane).unwrap();
        } else {
            let (panes, _) = pane_grid::State::new(pane);
            self.panes = Some(panes);
        }
        self.persist();
    }

    fn close_handle(&mut self, handle: pane_grid::Pane) {
        let Some(panes) = &mut self.panes else {
            return;
        };
        // Closing the last pane leaves no grid at all.
        if panes.iter().count() <= 1 {
            self.panes = None;
        } else {
            panes.close(handle);
        }
        self.persist();
    }

    /// The handle of the first open pane of `kind` in grid order, if any.
    fn handle_of_kind(&self, kind: DebuggerPane) -> Option<pane_grid::Pane> {
        self.panes
            .as_ref()?
            .iter()
            .find_map(|(&handle, pane)| (pane.kind() == kind).then_some(handle))
    }

    /// The sources currently shown by open instances of `kind`, so a new one
    /// can pick the first unshown.
    fn source_indices(&self, kind: DebuggerPane) -> Vec<usize> {
        let Some(panes) = &self.panes else {
            return Vec::new();
        };
        panes
            .iter()
            .filter(|(_, pane)| pane.kind() == kind)
            .filter_map(|(_, pane)| pane.source_index())
            .collect()
    }

    fn persist(&self) {
        // Tests exercise the same update path; keep their layout churn off the
        // user's real config file.
        #[cfg(not(test))]
        layout::save(self.family.layout_key, self.panes.as_ref());
        #[cfg(test)]
        let _ = self;
    }

    pub fn palette(&self) -> &Palette {
        self.palette.palette()
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.palette = palette;
        let policy = self.screen_palette_policy();
        self.with_screen_pane(|pane| pane.set_palette_policy(policy));
    }

    /// The colour policy the screen pane needs for this family and palette;
    /// `None` where the core resolves its own colour.
    fn screen_palette_policy(&self) -> Option<Box<dyn PalettePolicy>> {
        self.family
            .platforms
            .iter()
            .any(|platform| matches!(platform, Platform::GameBoy | Platform::GameBoyColor))
            .then(|| gb::dmg_palette_policy(self.palette, true))
    }

    /// The pane grid, rendered from the live console while paused or the
    /// per-vblank snapshot while the machine free-runs. Without a context (no
    /// readout or snapshot yet) panes show placeholders — except
    /// the screen, which always renders its own live frame.
    pub fn view<'a>(&'a self, ctx: Option<PaneContext<'_>>) -> Element<'a, app::Message> {
        if let Some(panes) = &self.panes {
            pane_grid(panes, move |handle, instance, _is_maximized| {
                instance.view(ctx.as_ref(), handle)
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
        self.instance_count(plane) > 0
    }

    /// How many instances of a kind are open — the rail badge count.
    pub fn instance_count(&self, kind: DebuggerPane) -> usize {
        match &self.panes {
            Some(panes) => panes.iter().filter(|(_, pane)| pane.kind() == kind).count(),
            None => 0,
        }
    }

    pub fn available_panes(&self) -> impl Iterator<Item = DebuggerPane> {
        self.family
            .registry
            .iter()
            .map(|descriptor| descriptor.kind)
    }

    /// Every open memory pane's selection, so the context builder can copy one
    /// window per instance and the running interest is their union.
    pub fn memory_selections(&self) -> Vec<MemorySelection> {
        let Some(panes) = &self.panes else {
            return Vec::new();
        };
        panes
            .iter()
            .filter_map(|(_, pane)| pane.memory_selection())
            .collect()
    }

    /// The disassembly pane's walk anchor, if that pane is shown and jumped
    /// somewhere; `None` follows the PC.
    pub fn disasm_anchor(&self) -> Option<u32> {
        self.panes
            .as_ref()?
            .iter()
            .find_map(|(_, pane)| pane.disasm_anchor())
    }
}

/// Ratios only change through resize drags, which have no end event to hook a
/// save to — persist the final layout when the debugger goes away instead.
impl Drop for DebuggerPanes {
    fn drop(&mut self) {
        self.persist();
    }
}

/// The first source no sibling shows: the lowest non-negative index absent from
/// `shown`. With every low index taken it keeps counting, so a surplus instance
/// lands past the sources and waits for the user's dropdown.
pub fn first_unshown_source(shown: impl IntoIterator<Item = usize>) -> usize {
    let taken: BTreeSet<usize> = shown.into_iter().collect();
    (0..).find(|i| !taken.contains(i)).unwrap()
}

pub fn running_placeholder(
    label: &str,
    close: pane_grid::Pane,
) -> pane_grid::Content<'_, app::Message> {
    pane(
        title_bar(label, close),
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

pub fn title_bar(label: &str, close: pane_grid::Pane) -> pane_grid::TitleBar<'_, app::Message> {
    build_title_bar(title_text(label), None, close)
}

/// A title bar with no close control, for the bottom panels that manage their
/// own open/close through the rail rather than the pane grid.
pub fn title_bar_plain(label: &str) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(container(title_text(label)).padding([xs(), s()])).style(title_style)
}

pub fn title_bar_with_detail<'a>(
    label: &'a str,
    detail: impl Into<Element<'a, app::Message>>,
    close: pane_grid::Pane,
) -> pane_grid::TitleBar<'a, app::Message> {
    build_title_bar(title_text(label), Some(detail.into()), close)
}

/// A compact `pick_list` sized and styled to sit in a pane title bar.
pub fn title_bar_picker<'a, T: ToString + PartialEq + Clone + 'a>(
    choices: Vec<T>,
    selected: Option<T>,
    on_select: impl Fn(T) -> app::Message + 'a,
) -> Element<'a, app::Message> {
    pick_list(choices, selected, on_select)
        .font(fonts::monospace())
        .text_size(11.0)
        .padding([1.0, 4.0])
        .into()
}

fn title_text(label: &str) -> Element<'_, app::Message> {
    iced::widget::text(label)
        .font(fonts::title())
        .size(13.0)
        .into()
}

fn build_title_bar<'a>(
    title: Element<'a, app::Message>,
    detail: Option<Element<'a, app::Message>>,
    close: pane_grid::Pane,
) -> pane_grid::TitleBar<'a, app::Message> {
    let mut controls = iced::widget::Row::new()
        .spacing(s())
        .align_y(iced::alignment::Vertical::Center);
    if let Some(detail) = detail {
        // +1px top padding nudge: the detail font (monospace 11px) is shorter
        // than the title font (Chakra Petch 13px), so it needs a small offset
        // to visually center within the title bar height.
        controls = controls.push(container(detail).padding([xs() + 1.0, 0.0]));
    }
    controls = controls.push(close_button(close));

    pane_grid::TitleBar::new(container(title).padding([xs(), s()]))
        .controls(Element::from(container(controls).padding([0.0, s()])))
        .always_show_controls()
        .style(title_style)
}

/// The title-bar close control every pane inherits, closing its own instance.
/// A small muted × that reads as quiet title-bar furniture — no chrome until
/// hovered — over a comfortably sized click target.
fn close_button<'a>(close: pane_grid::Pane) -> Element<'a, app::Message> {
    let glyph = iced::widget::text("\u{00D7}")
        .font(fonts::monospace())
        .size(11.0);
    button(glyph)
        .on_press(Message::CloseHandle(close).into())
        .style(close_control_style)
        .padding(xs() + 2.0)
        .into()
}

fn close_control_style(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: None,
        text_color: palette::MUTED,
        border: Border::default().rounded(4.0),
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(palette::PURPLE.scale_alpha(0.2).into()),
            ..base
        },
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_in(registry: &[PaneDescriptor]) -> Option<&PaneDescriptor> {
        registry
            .iter()
            .find(|descriptor| descriptor.kind == DebuggerPane::Audio)
    }

    #[test]
    fn audio_scope_registered_for_waveform_families() {
        // Both the Game Boy family and the VCS register the scope, under the
        // same "Audio" label so a layout saved by the old pane still resolves.
        for registry in [PANE_REGISTRY, VCS_PANE_REGISTRY] {
            let audio = audio_in(registry).expect("audio pane registered");
            assert_eq!(audio.label, "Audio");
        }
        // The pane's kind resolves back to that stable label.
        assert_eq!(DebuggerPane::Audio.to_string(), "Audio");
    }

    #[test]
    fn sg1000_registers_the_scope_and_the_graphics_surfaces() {
        let audio = audio_in(SG1000_PANE_REGISTRY).expect("audio pane registered");
        assert_eq!(audio.label, "Audio");
        // The VDP fills all three graphics surfaces, so all three are on the
        // rail, carrying the same labels a saved Game Boy layout uses.
        assert_graphics_panes(SG1000_PANE_REGISTRY);
    }

    /// The three graphics panes as a family registers them: same kinds, same
    /// labels, the two tile maps collapsed into one instanceable "Tile Map".
    fn assert_graphics_panes(registry: &[PaneDescriptor]) {
        let expected = [
            (DebuggerPane::Tiles, "Tiles", true),
            (DebuggerPane::TileMap, "Tile Map", true),
            (DebuggerPane::Sprites, "Sprites", false),
        ];
        for (kind, label, instanceable) in expected {
            let descriptor = registry
                .iter()
                .find(|descriptor| descriptor.kind == kind)
                .expect("graphics pane registered");
            assert_eq!(descriptor.label, label);
            assert_eq!(descriptor.instanceable, instanceable);
            assert_eq!(kind.to_string(), label);
        }
    }

    #[test]
    fn graphics_panes_keep_their_labels() {
        assert_graphics_panes(PANE_REGISTRY);
    }

    #[test]
    fn first_unshown_source_picks_lowest_absent() {
        // No siblings → the first source.
        assert_eq!(first_unshown_source([]), 0);
        // One sibling on source 0 → the next unshown.
        assert_eq!(first_unshown_source([0]), 1);
        // Both low sources taken (order-independent) → the one past them.
        assert_eq!(first_unshown_source([1, 0]), 2);
        // A gap is filled before counting on.
        assert_eq!(first_unshown_source([0, 2]), 1);
    }

    fn gb_panes() -> DebuggerPanes {
        DebuggerPanes {
            family: &GB_FAMILY,
            panes: None,
            palette: PaletteChoice::default(),
            screen_technology: ScreenView::new().technology(),
        }
    }

    #[test]
    fn instanceable_kind_opens_a_fresh_instance_each_click() {
        let mut panes = gb_panes();
        panes.update(Message::RailClick(DebuggerPane::TileMap));
        panes.update(Message::RailClick(DebuggerPane::TileMap));
        // Two tile maps coexist as distinct grid panes.
        assert_eq!(panes.instance_count(DebuggerPane::TileMap), 2);
        // Their default sources are the first two maps.
        let mut sources = panes.source_indices(DebuggerPane::TileMap);
        sources.sort_unstable();
        assert_eq!(sources, vec![0, 1]);
    }

    #[test]
    fn two_memory_instances_coexist_on_different_regions() {
        let mut panes = gb_panes();
        panes.update(Message::RailClick(DebuggerPane::Memory));
        panes.update(Message::RailClick(DebuggerPane::Memory));
        assert_eq!(panes.instance_count(DebuggerPane::Memory), 2);
        // Each defaulted to a different region; both selections are collected.
        let regions: Vec<usize> = panes
            .memory_selections()
            .iter()
            .map(|selection| selection.region)
            .collect();
        assert_eq!(regions.len(), 2);
        assert_ne!(regions[0], regions[1]);
    }

    #[test]
    fn single_instance_kind_toggles() {
        let mut panes = gb_panes();
        // Opening then clicking again closes it — today's toggle, unchanged.
        panes.update(Message::RailClick(DebuggerPane::Screen));
        assert!(panes.plane_shown(DebuggerPane::Screen));
        panes.update(Message::RailClick(DebuggerPane::Screen));
        assert!(!panes.plane_shown(DebuggerPane::Screen));
    }

    #[test]
    fn closing_last_instance_clears_the_badge() {
        let mut panes = gb_panes();
        panes.update(Message::RailClick(DebuggerPane::Tiles));
        panes.update(Message::RailClick(DebuggerPane::Tiles));
        let handle = panes.handle_of_kind(DebuggerPane::Tiles).unwrap();
        panes.update(Message::CloseHandle(handle));
        assert_eq!(panes.instance_count(DebuggerPane::Tiles), 1);
        let handle = panes.handle_of_kind(DebuggerPane::Tiles).unwrap();
        panes.update(Message::CloseHandle(handle));
        assert_eq!(panes.instance_count(DebuggerPane::Tiles), 0);
        assert!(!panes.plane_shown(DebuggerPane::Tiles));
    }

    /// The register-dump families expose only the screen and the two generic
    /// code/data panes; their chip state renders through the sidebar.
    #[cfg(any(feature = "nes", feature = "sms"))]
    fn assert_screen_and_generic_set(registry: &[PaneDescriptor]) {
        let kinds: Vec<DebuggerPane> = registry.iter().map(|descriptor| descriptor.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DebuggerPane::Screen,
                DebuggerPane::Memory,
                DebuggerPane::Disassembly,
            ]
        );
        for (kind, label) in [
            (DebuggerPane::Screen, "Screen"),
            (DebuggerPane::Memory, "Memory"),
            (DebuggerPane::Disassembly, "Disassembly"),
        ] {
            let descriptor = registry
                .iter()
                .find(|descriptor| descriptor.kind == kind)
                .expect("generic pane registered");
            assert_eq!(descriptor.label, label);
        }
    }

    #[cfg(feature = "nes")]
    #[test]
    fn nes_registry_is_screen_and_generic_set() {
        assert_screen_and_generic_set(NES_PANE_REGISTRY);
    }

    #[cfg(feature = "sms")]
    #[test]
    fn sms_registry_is_screen_and_generic_set() {
        assert_screen_and_generic_set(SMS_PANE_REGISTRY);
    }
}
