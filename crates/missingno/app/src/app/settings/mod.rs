pub(crate) mod update;
pub(crate) mod view;

use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    hash::Hash,
    path::PathBuf,
};

use missingno_core::ports::{
    ControlKind, PanelBehaviour, PeripheralDescriptor, PeripheralId, PortDescriptor, PortId,
    Provider,
};
use missingno_core::system::ControlRole;
use missingno_gb::ppu::types::palette::PaletteChoice;
use serde::{Deserialize, Serialize};

use crate::app::library::store::SortKey;
use crate::app::library::view::LibraryLayout;
use crate::app::system::{ControlMap, Platform, family_of};

// ── Bindable things ───────────────────────────────────────────────────

/// Emulator-level actions, not tied to any system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EmulatorAction {
    Screenshot,
    ToggleFullscreen,
    Pause,
    SaveState,
    LoadState,
    /// Start or stop capturing an input recording of the running game.
    ToggleRecording,
    /// Replay the game's recording slot.
    Replay,
}

/// The emulator actions, in display order.
pub const EMULATOR_ACTIONS: [EmulatorAction; 7] = [
    EmulatorAction::Screenshot,
    EmulatorAction::ToggleFullscreen,
    EmulatorAction::Pause,
    EmulatorAction::SaveState,
    EmulatorAction::LoadState,
    EmulatorAction::ToggleRecording,
    EmulatorAction::Replay,
];

impl fmt::Display for EmulatorAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EmulatorAction::Screenshot => "Screenshot",
            EmulatorAction::ToggleFullscreen => "Fullscreen",
            EmulatorAction::Pause => "Pause",
            EmulatorAction::SaveState => "Save State",
            EmulatorAction::LoadState => "Load State",
            EmulatorAction::ToggleRecording => "Record",
            EmulatorAction::Replay => "Replay",
        })
    }
}

/// Which way a held input turns a knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindDirection {
    Clockwise,
    CounterClockwise,
}

impl WindDirection {
    pub const BOTH: [WindDirection; 2] =
        [WindDirection::Clockwise, WindDirection::CounterClockwise];

    /// The row's name, read as the knob's own.
    pub fn label(self) -> &'static str {
        match self {
            WindDirection::Clockwise => "Knob clockwise",
            WindDirection::CounterClockwise => "Knob counterclockwise",
        }
    }

    pub fn id_name(self) -> &'static str {
        match self {
            WindDirection::Clockwise => "clockwise",
            WindDirection::CounterClockwise => "counterclockwise",
        }
    }
}

/// One bindable control on one system, addressed the way the seam declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlSlot {
    Integrated(ControlRole),
    Panel(ControlRole),
    /// One binding set per controller TYPE — deliberately no port dimension
    /// (per-port sets don't scale to multitaps; the source's port assignment
    /// picks where input lands).
    Peripheral {
        peripheral: PeripheralId,
        role: ControlRole,
    },
    /// Turning a controller's knob one way for as long as the input is held.
    Wind {
        peripheral: PeripheralId,
        role: ControlRole,
        direction: WindDirection,
    },
}

/// Which of a binding's two input surfaces is meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    Keyboard,
    Gamepad,
}

// ── Bindings ──────────────────────────────────────────────────────────

/// One input surface's bindings over one slot vocabulary. `bound` overrides
/// the defaults; `cleared` records an explicit unbinding, so a binding added
/// to the defaults later is still adopted.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(bound(
    serialize = "S: Serialize + Eq + Hash",
    deserialize = "S: Deserialize<'de> + Eq + Hash"
))]
pub struct InputMap<S> {
    #[serde(default)]
    bound: HashMap<S, String>,
    #[serde(default)]
    cleared: HashSet<S>,
}

impl<S> Default for InputMap<S> {
    fn default() -> Self {
        Self {
            bound: HashMap::new(),
            cleared: HashSet::new(),
        }
    }
}

impl<S: Copy + Eq + Hash> InputMap<S> {
    /// The key/button this slot answers to, defaults included.
    pub fn binding(&self, slot: S, defaults: &HashMap<S, String>) -> Option<String> {
        if let Some(key) = self.bound.get(&slot) {
            return Some(key.clone());
        }
        if self.cleared.contains(&slot) {
            return None;
        }
        defaults.get(&slot).cloned()
    }

    /// Every binding in force: the defaults, overlaid with `bound`, minus
    /// `cleared`.
    pub fn effective(&self, defaults: &HashMap<S, String>) -> HashMap<S, String> {
        let mut map = defaults.clone();
        for slot in &self.cleared {
            map.remove(slot);
        }
        for (slot, key) in &self.bound {
            map.insert(*slot, key.clone());
        }
        map
    }

    pub fn set(&mut self, slot: S, key: String) {
        self.cleared.remove(&slot);
        self.bound.insert(slot, key);
    }

    pub fn clear(&mut self, slot: S) {
        self.bound.remove(&slot);
        self.cleared.insert(slot);
    }

    pub fn reset(&mut self) {
        self.bound.clear();
        self.cleared.clear();
    }
}

/// The keyboard and gamepad bindings over one slot vocabulary.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(bound(
    serialize = "S: Serialize + Eq + Hash",
    deserialize = "S: Deserialize<'de> + Eq + Hash"
))]
pub struct SurfacePair<S> {
    #[serde(default)]
    pub keyboard: InputMap<S>,
    #[serde(default)]
    pub gamepad: InputMap<S>,
}

impl<S> Default for SurfacePair<S> {
    fn default() -> Self {
        Self {
            keyboard: InputMap::default(),
            gamepad: InputMap::default(),
        }
    }
}

impl<S> SurfacePair<S> {
    fn surface(&self, surface: Surface) -> &InputMap<S> {
        match surface {
            Surface::Keyboard => &self.keyboard,
            Surface::Gamepad => &self.gamepad,
        }
    }

    fn surface_mut(&mut self, surface: Surface) -> &mut InputMap<S> {
        match surface {
            Surface::Keyboard => &mut self.keyboard,
            Surface::Gamepad => &mut self.gamepad,
        }
    }
}

/// A gamepad as it can be recognised in a later session: its device uuid, plus
/// the driver's name for the pads whose uuid is unavailable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GamepadIdentity {
    pub uuid: [u8; 16],
    pub name: String,
}

impl GamepadIdentity {
    /// Whether the uuid tells this pad apart at all: a driver that reports none
    /// leaves it zeroed, and then only the name distinguishes devices.
    fn identifies(&self) -> bool {
        self.uuid != [0; 16]
    }
}

/// Which port each host device drove when this system was last played. The
/// keyboard's absence means it was never moved off the first port.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PersistedAssignments {
    #[serde(default)]
    pub keyboard: Option<PortId>,
    #[serde(default)]
    pub gamepads: Vec<(GamepadIdentity, PortId)>,
}

impl PersistedAssignments {
    /// The entries a pad answers to, in recorded order: the ones sharing its
    /// uuid where the driver reports one, and its name's otherwise. Reads and
    /// writes both go through this, so a pad is written back to the entry it
    /// was read from.
    fn recorded(&self, identity: &GamepadIdentity) -> Vec<usize> {
        let matching = |same: &dyn Fn(&GamepadIdentity) -> bool| {
            self.gamepads
                .iter()
                .enumerate()
                .filter(|(_, (known, _))| same(known))
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
        let by_uuid = match identity.identifies() {
            true => matching(&|known| known.uuid == identity.uuid),
            false => Vec::new(),
        };
        match by_uuid.is_empty() {
            true => matching(&|known| known.name == identity.name),
            false => by_uuid,
        }
    }

    /// The port this pad drove last time. `occurrence` counts identical twins
    /// in connection order, so a second pad of the same model takes the second
    /// entry recorded for it.
    pub fn port_for(&self, identity: &GamepadIdentity, occurrence: usize) -> Option<PortId> {
        let recorded = self.recorded(identity);
        recorded
            .get(occurrence)
            .map(|&index| self.gamepads[index].1)
    }

    /// Record where this pad plays, at the entry `port_for` reads. Twins that
    /// have never been placed take `default` so the occurrence index keeps
    /// counting the same entries.
    fn set_gamepad(
        &mut self,
        identity: GamepadIdentity,
        occurrence: usize,
        port: PortId,
        default: PortId,
    ) {
        let recorded = self.recorded(&identity);
        match recorded.get(occurrence) {
            // Written back with the identity it answers to now: a driver that
            // renames the device keeps its seat rather than gaining a second.
            Some(&index) => self.gamepads[index] = (identity, port),
            None => {
                for _ in recorded.len()..occurrence {
                    self.gamepads.push((identity.clone(), default));
                }
                self.gamepads.push((identity, port));
            }
        }
    }
}

/// Every binding the frontend holds: the emulator actions, one slot map per
/// system, and which host device played which port on each.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControlsSettings {
    #[serde(default)]
    pub emulator: SurfacePair<EmulatorAction>,
    #[serde(default)]
    pub systems: HashMap<Platform, SurfacePair<ControlSlot>>,
    #[serde(default)]
    pub assignments: HashMap<Platform, PersistedAssignments>,
    /// Whether the pointer over the screen turns the knob, per system. Absent
    /// means on.
    #[serde(default)]
    pub pointer_knob: HashMap<Platform, bool>,
}

impl ControlsSettings {
    pub fn emulator_binding(&self, surface: Surface, action: EmulatorAction) -> Option<String> {
        self.emulator
            .surface(surface)
            .binding(action, &default_emulator(surface))
    }

    /// The emulator actions in force on this surface, keyed by key/button.
    pub fn emulator_map(&self, surface: Surface) -> HashMap<EmulatorAction, String> {
        self.emulator
            .surface(surface)
            .effective(&default_emulator(surface))
    }

    pub fn system_binding(
        &self,
        platform: Platform,
        surface: Surface,
        slot: ControlSlot,
    ) -> Option<String> {
        match self.systems.get(&platform) {
            Some(pair) => pair
                .surface(surface)
                .binding(slot, &default_system(platform, surface)),
            None => default_system(platform, surface).get(&slot).cloned(),
        }
    }

    /// This system's bindings in force on this surface.
    pub fn system_map(&self, platform: Platform, surface: Surface) -> HashMap<ControlSlot, String> {
        let defaults = default_system(platform, surface);
        match self.systems.get(&platform) {
            Some(pair) => pair.surface(surface).effective(&defaults),
            None => defaults,
        }
    }

    pub fn set_emulator(&mut self, surface: Surface, action: EmulatorAction, key: String) {
        self.emulator.surface_mut(surface).set(action, key);
    }

    pub fn clear_emulator(&mut self, surface: Surface, action: EmulatorAction) {
        self.emulator.surface_mut(surface).clear(action);
    }

    pub fn set_system(
        &mut self,
        platform: Platform,
        surface: Surface,
        slot: ControlSlot,
        key: String,
    ) {
        self.systems
            .entry(platform)
            .or_default()
            .surface_mut(surface)
            .set(slot, key);
    }

    pub fn clear_system(&mut self, platform: Platform, surface: Surface, slot: ControlSlot) {
        self.systems
            .entry(platform)
            .or_default()
            .surface_mut(surface)
            .clear(slot);
    }

    pub fn reset_emulator(&mut self) {
        self.emulator.keyboard.reset();
        self.emulator.gamepad.reset();
    }

    /// Put a system's page back to its defaults — every switch it shows, not
    /// only its bindings.
    pub fn reset_system(&mut self, platform: Platform) {
        self.systems.remove(&platform);
        self.pointer_knob.remove(&platform);
    }

    /// Whether the pointer over the screen turns this system's knobs.
    pub fn pointer_knob(&self, platform: Platform) -> bool {
        self.pointer_knob.get(&platform).copied().unwrap_or(true)
    }

    pub fn set_pointer_knob(&mut self, platform: Platform, drives: bool) {
        self.pointer_knob.insert(platform, drives);
    }

    /// What this system remembers about which device played which port.
    pub fn assignments(&self, platform: Platform) -> Option<&PersistedAssignments> {
        self.assignments.get(&platform)
    }

    pub fn set_keyboard_port(&mut self, platform: Platform, port: PortId) {
        self.assignments.entry(platform).or_default().keyboard = Some(port);
    }

    /// Record which port a pad plays. `default` is where an unplaced twin sits
    /// — the machine's first port, which only the caller knows.
    pub fn set_gamepad_port(
        &mut self,
        platform: Platform,
        identity: GamepadIdentity,
        occurrence: usize,
        port: PortId,
        default: PortId,
    ) {
        self.assignments
            .entry(platform)
            .or_default()
            .set_gamepad(identity, occurrence, port, default);
    }
}

// ── Defaults ──────────────────────────────────────────────────────────

/// The emulator-action keys. No gamepad convention exists for these, so the
/// pad leaves them unbound for the user to place.
fn default_emulator(surface: Surface) -> HashMap<EmulatorAction, String> {
    match surface {
        Surface::Keyboard => HashMap::from([
            (EmulatorAction::Screenshot, "F12".to_string()),
            (EmulatorAction::ToggleFullscreen, "F11".to_string()),
            (EmulatorAction::Pause, "Space".to_string()),
            (EmulatorAction::SaveState, "F5".to_string()),
            (EmulatorAction::LoadState, "F8".to_string()),
            (EmulatorAction::ToggleRecording, "F6".to_string()),
            (EmulatorAction::Replay, "F7".to_string()),
        ]),
        Surface::Gamepad => HashMap::new(),
    }
}

/// The cluster every system's first controller gets, by role.
const KEYBOARD_CLUSTER: &[(ControlRole, &str)] = &[
    (ControlRole::Up, "ArrowUp"),
    (ControlRole::Down, "ArrowDown"),
    (ControlRole::Left, "ArrowLeft"),
    (ControlRole::Right, "ArrowRight"),
    (ControlRole::Action(0), "x"),
    (ControlRole::Action(1), "z"),
    (ControlRole::Start, "Enter"),
    (ControlRole::Select, "Shift"),
    (ControlRole::Reset, "Enter"),
    (ControlRole::Pause, "Enter"),
];

const GAMEPAD_CLUSTER: &[(ControlRole, &str)] = &[
    (ControlRole::Up, "DPadUp"),
    (ControlRole::Down, "DPadDown"),
    (ControlRole::Left, "DPadLeft"),
    (ControlRole::Right, "DPadRight"),
    (ControlRole::Action(0), "South"),
    (ControlRole::Action(1), "East"),
    (ControlRole::Start, "Start"),
    (ControlRole::Select, "Select"),
    (ControlRole::Reset, "Start"),
    (ControlRole::Pause, "Start"),
];

/// The keypad's 12 keys, row-major, on a 3×4 block of the typing keyboard that
/// keeps the pad's geometry. A frontend convention, following Stella's.
const KEYPAD_KEYS: [&str; 12] = ["1", "2", "3", "q", "w", "e", "a", "s", "d", "z", "x", "c"];

/// The pad's triggers wind a knob, squeeze depth setting the speed.
const WIND_TRIGGERS: [(WindDirection, &str); 2] = [
    (WindDirection::Clockwise, "RightTrigger2"),
    (WindDirection::CounterClockwise, "LeftTrigger2"),
];

/// The arrows wind at full rate, turned the way the on-screen paddle moves.
const WIND_ARROWS: [(WindDirection, &str); 2] = [
    (WindDirection::Clockwise, "ArrowRight"),
    (WindDirection::CounterClockwise, "ArrowLeft"),
];

/// This system's default bindings on one surface.
pub fn default_system(platform: Platform, surface: Surface) -> HashMap<ControlSlot, String> {
    let Some(family) = family_of(platform) else {
        return HashMap::new();
    };
    let cluster = match surface {
        Surface::Keyboard => KEYBOARD_CLUSTER,
        Surface::Gamepad => GAMEPAD_CLUSTER,
    };
    let mut map: HashMap<ControlSlot, String> = cluster
        .iter()
        .filter_map(|&(role, key)| {
            place(&family.controls, role).map(|slot| (slot, key.to_string()))
        })
        .collect();

    // The controllers beyond the one the cluster lands on: a keypad's keys and
    // the paddle's button.
    if platform == Platform::AtariVcs {
        use missingno_vcs::debug::{KEYPAD, PADDLES};
        let slot = |peripheral, role| ControlSlot::Peripheral { peripheral, role };
        if surface == Surface::Keyboard {
            for (index, key) in KEYPAD_KEYS.iter().enumerate() {
                map.insert(slot(KEYPAD, ControlRole::Key(index as u8)), key.to_string());
            }
        }
        let button = match surface {
            Surface::Keyboard => "x",
            Surface::Gamepad => "South",
        };
        map.insert(slot(PADDLES, ControlRole::Action(0)), button.to_string());
    }

    let winds = match surface {
        Surface::Keyboard => WIND_ARROWS,
        Surface::Gamepad => WIND_TRIGGERS,
    };
    for (peripheral, role) in knobs(platform) {
        for (direction, input) in winds {
            map.insert(
                ControlSlot::Wind {
                    peripheral,
                    role,
                    direction,
                },
                input.to_string(),
            );
        }
    }

    map
}

/// Every knob this system's controllers carry, as (controller, role).
pub fn knobs(platform: Platform) -> Vec<(PeripheralId, ControlRole)> {
    view::controller_types(platform)
        .into_iter()
        .flat_map(|peripheral| {
            peripheral
                .controls
                .iter()
                .filter(|control| control.kind == ControlKind::Axis)
                .map(|control| (peripheral.id, control.role))
        })
        .collect()
}

/// Where a family plays `role` in its default tables: the integrated pad, then
/// the controller its first port carries, then a panel button. Latching
/// switches are bindable but start unbound, and knobs are worked with the
/// pointer rather than a key.
fn place(controls: &ControlMap, role: ControlRole) -> Option<ControlSlot> {
    if controls.integrated.iter().any(|c| c.role == role) {
        return Some(ControlSlot::Integrated(role));
    }
    if let Some(port) = controls.ports.first()
        && let Some(peripheral) = default_peripheral(port)
        && peripheral.controls.iter().any(|c| c.role == role)
    {
        return Some(ControlSlot::Peripheral {
            peripheral: peripheral.id,
            role,
        });
    }
    controls
        .panel
        .iter()
        .any(|c| c.role == role && matches!(c.behaviour, PanelBehaviour::Momentary))
        .then_some(ControlSlot::Panel(role))
}

/// The controller a port carries in the default tables: the first one the core
/// builds that has any controls.
fn default_peripheral(port: &PortDescriptor) -> Option<&'static PeripheralDescriptor> {
    port.accepts.iter().find(|peripheral| {
        peripheral.provider == Provider::Console && !peripheral.controls.is_empty()
    })
}

// ── Settings persistence ──────────────────────────────────────────────

/// A `controls` block this build cannot read — one written against an older
/// slot vocabulary. Bindings carry no compatibility promise yet, so they fall
/// back to the defaults rather than costing every other setting in the file.
#[derive(Default)]
struct SkippedControls;

impl<'de> Deserialize<'de> for SkippedControls {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer).map(|_| SkippedControls)
    }
}

#[derive(Serialize, Deserialize)]
struct SettingsFile<C = ControlsSettings> {
    #[serde(default)]
    setup_complete: bool,
    #[serde(default)]
    internet_enabled: bool,
    #[serde(default = "default_true")]
    hasheous_enabled: bool,
    #[serde(default = "default_true")]
    homebrew_hub_enabled: bool,
    #[serde(default)]
    palette: String,
    #[serde(default)]
    rom_directories: Vec<PathBuf>,
    #[serde(default = "default_true")]
    use_sgb_colors: bool,
    // Renamed from the old `frame_blending` bool: a legacy file's value is
    // ignored (that global 50/50 was a different concept) and persistence
    // defaults on.
    #[serde(default = "default_true")]
    persistence: bool,
    // Cosmetic device-simulation options, keyed per display technology when
    // applied; default on for the authentic look, toggleable off.
    #[serde(default = "default_true")]
    pixel_grid: bool,
    #[serde(default = "default_true")]
    scanlines: bool,
    #[serde(default = "default_true")]
    cartridge_rw_enabled: bool,
    // Off unless the user opts in: with it off no socket exists, so nothing
    // outside this process can reach the running game.
    #[serde(default)]
    allow_external_clients: bool,
    // Off unless the user opts in: with it off no automation socket exists, so
    // nothing outside this process can enumerate or drive the window.
    #[serde(default)]
    allow_ui_automation: bool,
    #[serde(default)]
    library_sort: crate::app::library::store::SortKey,
    #[serde(default)]
    library_layout: crate::app::library::view::LibraryLayout,
    #[serde(default)]
    window_width: Option<f32>,
    #[serde(default)]
    window_height: Option<f32>,
    #[serde(default)]
    controls: C,
}

impl<C: Default> Default for SettingsFile<C> {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            hasheous_enabled: true,
            homebrew_hub_enabled: true,
            palette: palette_to_string(PaletteChoice::default()),
            rom_directories: Vec::new(),
            use_sgb_colors: true,
            persistence: true,
            pixel_grid: true,
            scanlines: true,
            cartridge_rw_enabled: true,
            allow_external_clients: false,
            allow_ui_automation: false,
            library_sort: SortKey::default(),
            library_layout: LibraryLayout::default(),
            window_width: None,
            window_height: None,
            controls: C::default(),
        }
    }
}

impl<C> SettingsFile<C> {
    /// Everything the file carries except the bindings, which the caller
    /// supplies — they are read separately so an unreadable block can fall back
    /// to the defaults.
    fn into_settings(self, controls: ControlsSettings) -> Settings {
        Settings {
            setup_complete: self.setup_complete,
            internet_enabled: self.internet_enabled,
            hasheous_enabled: self.hasheous_enabled,
            homebrew_hub_enabled: self.homebrew_hub_enabled,
            palette: parse_palette(&self.palette),
            rom_directories: self.rom_directories,
            use_sgb_colors: self.use_sgb_colors,
            persistence: self.persistence,
            pixel_grid: self.pixel_grid,
            scanlines: self.scanlines,
            cartridge_rw_enabled: self.cartridge_rw_enabled,
            allow_external_clients: self.allow_external_clients,
            allow_ui_automation: self.allow_ui_automation,
            library_sort: self.library_sort,
            library_layout: self.library_layout,
            window_width: self.window_width,
            window_height: self.window_height,
            controls,
        }
    }
}

fn default_true() -> bool {
    true
}

pub struct Settings {
    pub setup_complete: bool,
    pub internet_enabled: bool,
    pub hasheous_enabled: bool,
    pub homebrew_hub_enabled: bool,
    pub palette: PaletteChoice,
    pub rom_directories: Vec<PathBuf>,
    pub use_sgb_colors: bool,
    pub persistence: bool,
    pub pixel_grid: bool,
    pub scanlines: bool,
    pub cartridge_rw_enabled: bool,
    /// Whether a running game publishes an attach socket for clients in other
    /// processes (an agent driving the debugger).
    pub allow_external_clients: bool,
    /// Whether the app publishes a UI-automation socket for clients in other
    /// processes (an agent enumerating and driving the frontend).
    pub allow_ui_automation: bool,
    pub library_sort: SortKey,
    pub library_layout: LibraryLayout,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    pub controls: ControlsSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            hasheous_enabled: true,
            homebrew_hub_enabled: true,
            palette: PaletteChoice::default(),
            rom_directories: Vec::new(),
            use_sgb_colors: true,
            persistence: true,
            pixel_grid: true,
            scanlines: true,
            cartridge_rw_enabled: true,
            allow_external_clients: false,
            allow_ui_automation: false,
            library_sort: SortKey::default(),
            library_layout: LibraryLayout::default(),
            window_width: None,
            window_height: None,
            controls: ControlsSettings::default(),
        }
    }
}

impl Settings {
    /// The display-presentation snapshot handed to frame captures.
    pub fn capture_options(&self) -> crate::app::library::activity::CaptureOptions {
        crate::app::library::activity::CaptureOptions {
            use_sgb_colors: self.use_sgb_colors,
            palette_name: self.palette.to_string(),
        }
    }

    /// The display options the settings screen offers: every effect, and the
    /// Game Boy colours, whatever console is loaded.
    pub fn display_options(&self) -> view::DisplayOptions {
        view::DisplayOptions {
            effects: view::Effects {
                persistence: self.persistence,
                scanlines: self.scanlines,
                pixel_grid: self.pixel_grid,
            },
            technology: None,
            sgb_colors: Some(self.use_sgb_colors),
            palette: Some(self.palette),
        }
    }

    /// The renderer's presentation choices, from the display settings.
    pub fn presentation(&self) -> crate::app::emulator::Presentation {
        crate::app::emulator::Presentation {
            use_sgb_colors: self.use_sgb_colors,
            persistence: self.persistence,
            pixel_grid: self.pixel_grid,
            scanlines: self.scanlines,
        }
    }

    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };

        let Ok(data) = fs::read_to_string(&path) else {
            return Self::default();
        };

        Self::parse(&data).unwrap_or_default()
    }

    /// A settings file's contents. A file written before a field existed takes
    /// that field's default and fields this build no longer knows are ignored;
    /// a `controls` block it cannot read costs the bindings alone.
    fn parse(data: &str) -> Option<Self> {
        if let Ok(mut file) = ron::from_str::<SettingsFile>(data) {
            let controls = std::mem::take(&mut file.controls);
            return Some(file.into_settings(controls));
        }
        ron::from_str::<SettingsFile<SkippedControls>>(data)
            .ok()
            .map(|file| file.into_settings(ControlsSettings::default()))
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let file = SettingsFile {
            setup_complete: self.setup_complete,
            internet_enabled: self.internet_enabled,
            hasheous_enabled: self.hasheous_enabled,
            homebrew_hub_enabled: self.homebrew_hub_enabled,
            palette: palette_to_string(self.palette),
            rom_directories: self.rom_directories.clone(),
            use_sgb_colors: self.use_sgb_colors,
            persistence: self.persistence,
            pixel_grid: self.pixel_grid,
            scanlines: self.scanlines,
            cartridge_rw_enabled: self.cartridge_rw_enabled,
            allow_external_clients: self.allow_external_clients,
            allow_ui_automation: self.allow_ui_automation,
            library_sort: self.library_sort,
            library_layout: self.library_layout,
            window_width: self.window_width,
            window_height: self.window_height,
            controls: self.controls.clone(),
        };
        if let Ok(data) = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default()) {
            let _ = fs::write(path, data);
        }
    }
}

fn parse_palette(value: &str) -> PaletteChoice {
    match value {
        "Green" => PaletteChoice::Green,
        "Pocket" => PaletteChoice::Pocket,
        "Classic" => PaletteChoice::Classic,
        _ => PaletteChoice::default(),
    }
}

fn palette_to_string(palette: PaletteChoice) -> String {
    match palette {
        PaletteChoice::Green => "Green",
        PaletteChoice::Pocket => "Pocket",
        PaletteChoice::Classic => "Classic",
    }
    .to_string()
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("settings.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_vcs::debug::{JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, RIGHT_PORT};

    fn controller_slot(peripheral: PeripheralId, role: ControlRole) -> ControlSlot {
        ControlSlot::Peripheral { peripheral, role }
    }

    /// Winding the VCS paddle's knob, the only knob any registered system has.
    fn wind_slot(direction: WindDirection) -> ControlSlot {
        ControlSlot::Wind {
            peripheral: PADDLES,
            role: ControlRole::Knob(0),
            direction,
        }
    }

    fn pad(uuid: u8, name: &str) -> GamepadIdentity {
        GamepadIdentity {
            uuid: [uuid; 16],
            name: name.to_string(),
        }
    }

    #[test]
    fn a_pre_rework_settings_file_loads_with_default_bindings() {
        // Exactly the shape an upgraded install carries: bindings keyed by the
        // old platform-ambiguous actions, plus the action-set marker.
        let old = r#"(
            setup_complete: true,
            internet_enabled: true,
            palette: "Pocket",
            rom_directories: ["/home/test/roms"],
            use_sgb_colors: false,
            window_width: Some(1920.0),
            keyboard_controls: ({
                Control(Action(0)): "q",
                Screenshot: "F9",
            }),
            gamepad_controls: ({
                Control(Action(0)): "West",
            }),
            known_actions: [Screenshot, Pause],
            keyboard_bindings: (a: "x", b: "z"),
        )"#;

        let file: SettingsFile = ron::from_str(old).expect("old settings parse");
        assert!(file.setup_complete);
        assert_eq!(file.palette, "Pocket");
        assert_eq!(file.rom_directories, vec![PathBuf::from("/home/test/roms")]);
        assert!(!file.use_sgb_colors);
        assert_eq!(file.window_width, Some(1920.0));

        // The old binding fields are unknown to this build: bindings are the
        // defaults, not what that file carried.
        assert!(file.controls.systems.is_empty());
        assert_eq!(
            file.controls
                .emulator_binding(Surface::Keyboard, EmulatorAction::Screenshot),
            Some("F12".to_string())
        );
        assert_eq!(
            file.controls.system_binding(
                Platform::GameBoy,
                Surface::Keyboard,
                ControlSlot::Integrated(ControlRole::Action(0))
            ),
            Some("x".to_string())
        );
    }

    #[test]
    fn bindings_round_trip_through_the_settings_file() {
        let mut controls = ControlsSettings::default();
        controls.set_emulator(
            Surface::Keyboard,
            EmulatorAction::Screenshot,
            "F9".to_string(),
        );
        controls.clear_emulator(Surface::Keyboard, EmulatorAction::Replay);
        controls.set_system(
            Platform::AtariVcs,
            Surface::Keyboard,
            controller_slot(KEYPAD, ControlRole::Key(11)),
            "m".to_string(),
        );
        controls.set_system(
            Platform::GameBoyColor,
            Surface::Gamepad,
            ControlSlot::Integrated(ControlRole::Start),
            "North".to_string(),
        );
        controls.set_system(
            Platform::AtariVcs,
            Surface::Keyboard,
            ControlSlot::Panel(ControlRole::Reset),
            "F1".to_string(),
        );
        controls.clear_system(
            Platform::AtariVcs,
            Surface::Keyboard,
            controller_slot(JOYSTICK, ControlRole::Action(0)),
        );
        controls.set_system(
            Platform::AtariVcs,
            Surface::Keyboard,
            wind_slot(WindDirection::CounterClockwise),
            "Comma".to_string(),
        );
        controls.set_pointer_knob(Platform::AtariVcs, false);
        controls.set_keyboard_port(Platform::AtariVcs, RIGHT_PORT);
        controls.set_gamepad_port(
            Platform::AtariVcs,
            pad(7, "Wireless Pad"),
            0,
            LEFT_PORT,
            LEFT_PORT,
        );
        controls.set_gamepad_port(
            Platform::AtariVcs,
            pad(7, "Wireless Pad"),
            1,
            RIGHT_PORT,
            LEFT_PORT,
        );

        let file = SettingsFile {
            controls,
            ..SettingsFile::default()
        };
        let written =
            ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default()).expect("write");
        let read: SettingsFile = ron::from_str(&written).expect("re-read");

        assert_eq!(
            read.controls
                .emulator_binding(Surface::Keyboard, EmulatorAction::Screenshot),
            Some("F9".to_string())
        );
        assert_eq!(
            read.controls
                .emulator_binding(Surface::Keyboard, EmulatorAction::Replay),
            None
        );
        assert_eq!(
            read.controls.system_binding(
                Platform::AtariVcs,
                Surface::Keyboard,
                controller_slot(KEYPAD, ControlRole::Key(11))
            ),
            Some("m".to_string())
        );
        assert_eq!(
            read.controls.system_binding(
                Platform::AtariVcs,
                Surface::Keyboard,
                ControlSlot::Panel(ControlRole::Reset)
            ),
            Some("F1".to_string())
        );
        assert_eq!(
            read.controls.system_binding(
                Platform::AtariVcs,
                Surface::Keyboard,
                controller_slot(JOYSTICK, ControlRole::Action(0))
            ),
            None
        );
        assert_eq!(
            read.controls.system_binding(
                Platform::GameBoyColor,
                Surface::Gamepad,
                ControlSlot::Integrated(ControlRole::Start)
            ),
            Some("North".to_string())
        );
        // The wind slots and the pointer switch come back with the rest.
        assert_eq!(
            read.controls.system_binding(
                Platform::AtariVcs,
                Surface::Keyboard,
                wind_slot(WindDirection::CounterClockwise)
            ),
            Some("Comma".to_string())
        );
        assert_eq!(
            read.controls.system_binding(
                Platform::AtariVcs,
                Surface::Gamepad,
                wind_slot(WindDirection::Clockwise)
            ),
            Some("RightTrigger2".to_string())
        );
        assert!(!read.controls.pointer_knob(Platform::AtariVcs));
        assert!(read.controls.pointer_knob(Platform::GameBoy));

        // Another system's map is untouched by either edit.
        assert_eq!(
            read.controls.system_binding(
                Platform::GameBoy,
                Surface::Keyboard,
                ControlSlot::Integrated(ControlRole::Start)
            ),
            Some("Enter".to_string())
        );

        // The ports the devices played come back with them, twins included.
        let seating = read
            .controls
            .assignments(Platform::AtariVcs)
            .expect("VCS seating");
        assert_eq!(seating.keyboard, Some(RIGHT_PORT));
        assert_eq!(
            seating.port_for(&pad(7, "Wireless Pad"), 0),
            Some(LEFT_PORT)
        );
        assert_eq!(
            seating.port_for(&pad(7, "Wireless Pad"), 1),
            Some(RIGHT_PORT)
        );
        assert!(read.controls.assignments(Platform::GameBoy).is_none());
    }

    #[test]
    fn a_pad_is_recognised_by_uuid_first_then_by_name() {
        let mut controls = ControlsSettings::default();
        controls.set_gamepad_port(
            Platform::AtariVcs,
            pad(1, "Pad One"),
            0,
            RIGHT_PORT,
            LEFT_PORT,
        );
        // A pad whose driver reports no uuid is known by name alone.
        controls.set_gamepad_port(
            Platform::AtariVcs,
            pad(0, "Nameless Clone"),
            0,
            LEFT_PORT,
            LEFT_PORT,
        );
        let seating = controls.assignments(Platform::AtariVcs).unwrap();

        // The uuid wins even when the driver renames the device.
        assert_eq!(
            seating.port_for(
                &GamepadIdentity {
                    uuid: [1; 16],
                    name: "Pad One (2)".to_string()
                },
                0
            ),
            Some(RIGHT_PORT)
        );
        // A different pad reporting the same name falls back to the name match.
        assert_eq!(seating.port_for(&pad(9, "Pad One"), 0), Some(RIGHT_PORT));
        // A zeroed uuid never stands in for identity: two uuid-less pads of
        // different models stay distinct.
        assert_eq!(seating.port_for(&pad(0, "Other Clone"), 0), None);
        assert_eq!(
            seating.port_for(&pad(0, "Nameless Clone"), 0),
            Some(missingno_vcs::debug::LEFT_PORT)
        );
        // Nothing recorded for a second twin: the caller falls back to the
        // machine's first port.
        assert_eq!(seating.port_for(&pad(1, "Pad One"), 1), None);
    }

    #[test]
    fn twins_seated_out_of_order_keep_their_own_ports() {
        let mut controls = ControlsSettings::default();
        // The second of two identical pads is moved first: the first twin's
        // entry is filled in on the way, at the machine's first port.
        controls.set_gamepad_port(Platform::AtariVcs, pad(3, "Twin"), 1, RIGHT_PORT, LEFT_PORT);
        controls.set_gamepad_port(Platform::AtariVcs, pad(3, "Twin"), 0, LEFT_PORT, LEFT_PORT);

        let seating = controls.assignments(Platform::AtariVcs).unwrap();
        assert_eq!(seating.port_for(&pad(3, "Twin"), 0), Some(LEFT_PORT));
        assert_eq!(seating.port_for(&pad(3, "Twin"), 1), Some(RIGHT_PORT));
        assert_eq!(seating.gamepads.len(), 2);
    }

    #[test]
    fn a_renamed_pad_keeps_its_seat_rather_than_gaining_a_second() {
        let mut controls = ControlsSettings::default();
        controls.set_gamepad_port(Platform::AtariVcs, pad(5, "Pad"), 0, LEFT_PORT, LEFT_PORT);
        // Same device, renamed by its driver: the uuid still finds the entry.
        let renamed = GamepadIdentity {
            uuid: [5; 16],
            name: "Pad (2)".to_string(),
        };
        controls.set_gamepad_port(
            Platform::AtariVcs,
            renamed.clone(),
            0,
            RIGHT_PORT,
            LEFT_PORT,
        );

        let seating = controls.assignments(Platform::AtariVcs).unwrap();
        assert_eq!(seating.gamepads.len(), 1);
        assert_eq!(seating.port_for(&renamed, 0), Some(RIGHT_PORT));
        // The stored identity is the fresh one, so the name match follows too.
        assert_eq!(seating.gamepads[0].0.name, "Pad (2)");
    }

    #[test]
    fn a_cleared_slot_stays_cleared_and_a_new_default_still_arrives() {
        let mut map: InputMap<ControlSlot> = InputMap::default();
        let start = ControlSlot::Integrated(ControlRole::Start);
        let select = ControlSlot::Integrated(ControlRole::Select);
        map.clear(start);

        let defaults = default_system(Platform::GameBoy, Surface::Keyboard);
        let effective = map.effective(&defaults);
        assert!(!effective.contains_key(&start));
        // A slot the user never touched keeps its default, so a binding added
        // to the defaults later is adopted.
        assert_eq!(effective.get(&select), Some(&"Shift".to_string()));

        // Binding it again un-clears it.
        map.set(start, "Tab".to_string());
        assert_eq!(map.binding(start, &defaults), Some("Tab".to_string()));
    }

    #[test]
    fn game_boy_defaults_sit_on_the_integrated_pad() {
        let keyboard = default_system(Platform::GameBoy, Surface::Keyboard);
        assert_eq!(
            keyboard.get(&ControlSlot::Integrated(ControlRole::Up)),
            Some(&"ArrowUp".to_string())
        );
        assert_eq!(
            keyboard.get(&ControlSlot::Integrated(ControlRole::Action(1))),
            Some(&"z".to_string())
        );
        // The Game Boy has no panel and no controllable port.
        assert!(
            keyboard
                .keys()
                .all(|slot| matches!(slot, ControlSlot::Integrated(_)))
        );

        let gamepad = default_system(Platform::GameBoy, Surface::Gamepad);
        assert_eq!(
            gamepad.get(&ControlSlot::Integrated(ControlRole::Action(0))),
            Some(&"South".to_string())
        );
    }

    #[test]
    fn vcs_defaults_cover_each_controller_type_and_the_panel() {
        let keyboard = default_system(Platform::AtariVcs, Surface::Keyboard);
        assert_eq!(
            keyboard.get(&ControlSlot::Panel(ControlRole::Reset)),
            Some(&"Enter".to_string())
        );
        assert_eq!(
            keyboard.get(&ControlSlot::Panel(ControlRole::Select)),
            Some(&"Shift".to_string())
        );
        // Latching switches are bindable, but start unbound.
        assert!(!keyboard.contains_key(&ControlSlot::Panel(ControlRole::Toggle(0))));
        // A knob binds no position; the arrows wind it the way the paddle moves.
        assert!(!keyboard.contains_key(&controller_slot(PADDLES, ControlRole::Knob(0))));
        assert_eq!(
            keyboard.get(&wind_slot(WindDirection::Clockwise)),
            Some(&"ArrowRight".to_string())
        );
        assert_eq!(
            keyboard.get(&wind_slot(WindDirection::CounterClockwise)),
            Some(&"ArrowLeft".to_string())
        );

        // The bindings key on the controller, not the jack it sits in: the
        // device assignment picks which player a device plays.
        assert_eq!(
            keyboard.get(&controller_slot(JOYSTICK, ControlRole::Action(0))),
            Some(&"x".to_string())
        );
        assert_eq!(
            keyboard.get(&controller_slot(JOYSTICK, ControlRole::Up)),
            Some(&"ArrowUp".to_string())
        );
        // One paddle per jack is surfaced, so it takes the same button as the
        // joystick that would otherwise be in that port.
        assert_eq!(
            keyboard.get(&controller_slot(PADDLES, ControlRole::Action(0))),
            Some(&"x".to_string())
        );
        assert!(!keyboard.contains_key(&controller_slot(PADDLES, ControlRole::Action(1))));
        // The keypad keeps its 3×4 geometry.
        assert_eq!(
            keyboard.get(&controller_slot(KEYPAD, ControlRole::Key(0))),
            Some(&"1".to_string())
        );
        assert_eq!(
            keyboard.get(&controller_slot(KEYPAD, ControlRole::Key(4))),
            Some(&"w".to_string())
        );
        assert_eq!(
            keyboard.get(&controller_slot(KEYPAD, ControlRole::Key(11))),
            Some(&"c".to_string())
        );

        let gamepad = default_system(Platform::AtariVcs, Surface::Gamepad);
        assert_eq!(
            gamepad.get(&controller_slot(JOYSTICK, ControlRole::Up)),
            Some(&"DPadUp".to_string())
        );
        assert_eq!(
            gamepad.get(&controller_slot(JOYSTICK, ControlRole::Action(0))),
            Some(&"South".to_string())
        );
        // The triggers wind the knob, the way the differential winder reads
        // them.
        assert_eq!(
            gamepad.get(&wind_slot(WindDirection::Clockwise)),
            Some(&"RightTrigger2".to_string())
        );
        assert_eq!(
            gamepad.get(&wind_slot(WindDirection::CounterClockwise)),
            Some(&"LeftTrigger2".to_string())
        );
    }

    #[test]
    fn the_pointer_turns_the_knob_until_it_is_switched_off() {
        let mut controls = ControlsSettings::default();
        assert!(controls.pointer_knob(Platform::AtariVcs));
        controls.set_pointer_knob(Platform::AtariVcs, false);
        assert!(!controls.pointer_knob(Platform::AtariVcs));
        // The switch is per system.
        assert!(controls.pointer_knob(Platform::GameBoy));
    }

    // One press must not work two controls at once. A device drives the console
    // itself and the one controller in the port it plays, so those slots share
    // an input space; two controller TYPES never answer together, and the
    // default clusters deliberately reuse keys across them.
    #[test]
    fn no_default_binds_one_input_to_two_controls_a_device_reaches_together() {
        // The controller a slot belongs to; the console's own controls belong
        // to none and answer whatever is plugged.
        fn controller(slot: &ControlSlot) -> Option<PeripheralId> {
            match slot {
                ControlSlot::Peripheral { peripheral, .. }
                | ControlSlot::Wind { peripheral, .. } => Some(*peripheral),
                _ => None,
            }
        }

        for family in crate::app::system::FAMILIES {
            for surface in [Surface::Keyboard, Surface::Gamepad] {
                let defaults = default_system(family.platform, surface);
                let peripherals: Vec<Option<PeripheralId>> = std::iter::once(None)
                    .chain(
                        defaults
                            .keys()
                            .filter_map(controller)
                            .map(Some)
                            .collect::<HashSet<_>>(),
                    )
                    .collect();

                for reachable in peripherals {
                    let mut by_input: HashMap<&String, Vec<&ControlSlot>> = HashMap::new();
                    for (slot, input) in &defaults {
                        let together = match controller(slot) {
                            Some(peripheral) => Some(peripheral) == reachable,
                            None => true,
                        };
                        if together {
                            by_input.entry(input).or_default().push(slot);
                        }
                    }
                    for (input, slots) in by_input {
                        assert!(
                            slots.len() == 1,
                            "{} {surface:?} binds {input} to {slots:?}",
                            family.platform
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_binding_this_build_cannot_read_costs_only_the_bindings() {
        // Bindings written against the port-keyed slot vocabulary: the whole
        // file still loads, with the bindings back at their defaults.
        let stale = r#"(
            setup_complete: true,
            rom_directories: ["/home/test/roms"],
            window_width: Some(1280.0),
            controls: (
                systems: {
                    AtariVcs: (
                        keyboard: (
                            bound: { Port(port: (1), peripheral: (3), role: Key(11)): "m" },
                            cleared: [],
                        ),
                        gamepad: (bound: {}, cleared: []),
                    ),
                },
            ),
        )"#;

        let settings = Settings::parse(stale).expect("stale bindings do not cost the file");
        assert!(settings.setup_complete);
        assert_eq!(
            settings.rom_directories,
            vec![PathBuf::from("/home/test/roms")]
        );
        assert_eq!(settings.window_width, Some(1280.0));
        assert!(settings.controls.systems.is_empty());
        assert_eq!(
            settings.controls.system_binding(
                Platform::AtariVcs,
                Surface::Keyboard,
                controller_slot(KEYPAD, ControlRole::Key(11))
            ),
            Some("c".to_string())
        );
    }

    #[test]
    fn legacy_frame_blending_maps_to_persistence_on() {
        // The old global 50/50 `frame_blending` bool is a different concept from
        // the new persistence switch: whatever value a file carries, persistence
        // migrates on (its default).
        let old = r#"(setup_complete: true, frame_blending: false)"#;
        let file: SettingsFile = ron::from_str(old).unwrap();
        assert!(file.persistence);
        // A file already carrying the new key is respected.
        let current = r#"(setup_complete: true, persistence: false)"#;
        let file: SettingsFile = ron::from_str(current).unwrap();
        assert!(!file.persistence);
    }

    #[test]
    fn overlay_options_default_on() {
        // A file predating the cosmetic overlays gets both on — the authentic
        // look is the default.
        let old = r#"(setup_complete: true)"#;
        let file: SettingsFile = ron::from_str(old).unwrap();
        assert!(file.pixel_grid);
        assert!(file.scanlines);
        // An explicit off round-trips.
        let set = r#"(setup_complete: true, pixel_grid: false, scanlines: false)"#;
        let file: SettingsFile = ron::from_str(set).unwrap();
        assert!(!file.pixel_grid);
        assert!(!file.scanlines);
    }

    #[test]
    fn external_clients_round_trip() {
        let opted_in = r#"( setup_complete: true, allow_external_clients: true )"#;
        let file: SettingsFile = ron::from_str(opted_in).unwrap();
        assert!(file.allow_external_clients);

        let written = ron::ser::to_string(&file).unwrap();
        let reread: SettingsFile = ron::from_str(&written).unwrap();
        assert!(reread.allow_external_clients);
    }
}
