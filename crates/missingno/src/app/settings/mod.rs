pub(crate) mod update;
pub(crate) mod view;

use std::{collections::HashMap, fmt, fs, path::PathBuf};

use missingno_gb::ppu::types::palette::PaletteChoice;
use serde::{Deserialize, Serialize};

use crate::app::library::store::SortKey;
use crate::app::library::view::LibraryLayout;

// ── Actions ───────────────────────────────────────────────────────────

/// Every bindable action — game controls and emulator controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// A shared seam control id (press/release), interpreted per family.
    Control(u8),
    // Emulator actions (fire once on press)
    Screenshot,
    ToggleFullscreen,
    Pause,
}

/// The 8 shared game controls in display order (see `ControlId` for the
/// id convention).
pub const GAME_CONTROLS: [Action; 8] = [
    Action::Control(4), // Up
    Action::Control(5), // Down
    Action::Control(6), // Left
    Action::Control(7), // Right
    Action::Control(2), // A
    Action::Control(3), // B
    Action::Control(0), // Start
    Action::Control(1), // Select
];

/// Emulator-level actions, for iteration.
pub const EMULATOR_ACTIONS: [Action; 3] =
    [Action::Screenshot, Action::ToggleFullscreen, Action::Pause];

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Family-neutral; the bindings UI derives each family's name for
            // a control from the descriptor table.
            Action::Control(id) => write!(f, "Control {id}"),
            Action::Screenshot => write!(f, "Screenshot"),
            Action::ToggleFullscreen => write!(f, "Fullscreen"),
            Action::Pause => write!(f, "Pause"),
        }
    }
}

// ── Bindings ──────────────────────────────────────────────────────────

/// Map of action → key/button string. One instance for keyboard, one for gamepad.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bindings(pub HashMap<Action, String>);

impl Bindings {
    pub fn get(&self, action: Action) -> Option<&str> {
        self.0.get(&action).map(|s| s.as_str())
    }

    pub fn set(&mut self, action: Action, value: String) {
        self.0.insert(action, value);
    }

    /// Find the action bound to a given key/button string.
    pub fn find_action(&self, key_str: &str) -> Option<Action> {
        self.0
            .iter()
            .find(|(_, v)| v.as_str() == key_str)
            .map(|(k, _)| *k)
    }

    pub fn default_keyboard() -> Self {
        Self(HashMap::from([
            (Action::Control(2), "x".to_string()),
            (Action::Control(3), "z".to_string()),
            (Action::Control(0), "Enter".to_string()),
            (Action::Control(1), "Shift".to_string()),
            (Action::Control(4), "ArrowUp".to_string()),
            (Action::Control(5), "ArrowDown".to_string()),
            (Action::Control(6), "ArrowLeft".to_string()),
            (Action::Control(7), "ArrowRight".to_string()),
            (Action::Screenshot, "F12".to_string()),
            (Action::ToggleFullscreen, "F11".to_string()),
            (Action::Pause, "Space".to_string()),
        ]))
    }

    pub fn default_gamepad() -> Self {
        Self(HashMap::from([
            (Action::Control(2), "South".to_string()),
            (Action::Control(3), "East".to_string()),
            (Action::Control(0), "Start".to_string()),
            (Action::Control(1), "Select".to_string()),
            (Action::Control(4), "DPadUp".to_string()),
            (Action::Control(5), "DPadDown".to_string()),
            (Action::Control(6), "DPadLeft".to_string()),
            (Action::Control(7), "DPadRight".to_string()),
            // Emulator actions unbound by default on gamepad —
            // no standard convention, user picks what suits their device.
        ]))
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Self::default_keyboard()
    }
}

// ── Settings persistence ──────────────────────────────────────────────

// Legacy flat struct for migrating old settings files.
#[derive(Deserialize, Default)]
struct LegacyKeyBindings {
    #[serde(default)]
    a: String,
    #[serde(default)]
    b: String,
    #[serde(default)]
    start: String,
    #[serde(default)]
    select: String,
    #[serde(default)]
    up: String,
    #[serde(default)]
    down: String,
    #[serde(default)]
    left: String,
    #[serde(default)]
    right: String,
}

impl From<LegacyKeyBindings> for Bindings {
    fn from(old: LegacyKeyBindings) -> Self {
        // Only migrate the game button bindings — emulator action defaults
        // are added by the caller based on whether this is keyboard or gamepad.
        let mut map = HashMap::new();
        for (id, value) in [
            (2, old.a),
            (3, old.b),
            (0, old.start),
            (1, old.select),
            (4, old.up),
            (5, old.down),
            (6, old.left),
            (7, old.right),
        ] {
            if !value.is_empty() {
                map.insert(Action::Control(id), value);
            }
        }
        Bindings(map)
    }
}

/// The pre-control-id action names, for migrating bindings keyed by them.
#[derive(Deserialize, PartialEq, Eq, Hash)]
enum LegacyAction {
    GbA,
    GbB,
    GbStart,
    GbSelect,
    GbUp,
    GbDown,
    GbLeft,
    GbRight,
    Screenshot,
    ToggleFullscreen,
    Pause,
}

#[derive(Deserialize, Default)]
struct LegacyActionBindings(HashMap<LegacyAction, String>);

impl From<LegacyActionBindings> for Bindings {
    fn from(old: LegacyActionBindings) -> Self {
        Bindings(
            old.0
                .into_iter()
                .map(|(action, value)| {
                    let action = match action {
                        LegacyAction::GbStart => Action::Control(0),
                        LegacyAction::GbSelect => Action::Control(1),
                        LegacyAction::GbA => Action::Control(2),
                        LegacyAction::GbB => Action::Control(3),
                        LegacyAction::GbUp => Action::Control(4),
                        LegacyAction::GbDown => Action::Control(5),
                        LegacyAction::GbLeft => Action::Control(6),
                        LegacyAction::GbRight => Action::Control(7),
                        LegacyAction::Screenshot => Action::Screenshot,
                        LegacyAction::ToggleFullscreen => Action::ToggleFullscreen,
                        LegacyAction::Pause => Action::Pause,
                    };
                    (action, value)
                })
                .collect(),
        )
    }
}

fn default_keyboard_bindings() -> Bindings {
    Bindings::default_keyboard()
}
fn default_gamepad_bindings() -> Bindings {
    Bindings::default_gamepad()
}

#[derive(Serialize, Deserialize)]
struct SettingsFile {
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
    #[serde(default)]
    library_sort: crate::app::library::store::SortKey,
    #[serde(default)]
    library_layout: crate::app::library::view::LibraryLayout,
    #[serde(default)]
    window_width: Option<f32>,
    #[serde(default)]
    window_height: Option<f32>,
    #[serde(default = "default_keyboard_bindings")]
    keyboard_controls: Bindings,
    #[serde(default = "default_gamepad_bindings")]
    gamepad_controls: Bindings,
    // Bindings keyed by the pre-control-id action names; read for
    // migration, never written. Empty when the file is current-format.
    #[serde(default, skip_serializing)]
    keyboard_bindings: LegacyActionBindings,
    #[serde(default, skip_serializing)]
    gamepad_bindings: LegacyActionBindings,
}

/// Legacy settings file format with flat KeyBindings structs.
#[derive(Deserialize)]
struct LegacySettingsFile {
    #[serde(default)]
    setup_complete: bool,
    #[serde(default)]
    internet_enabled: bool,
    #[serde(default)]
    palette: String,
    #[serde(default)]
    rom_directories: Vec<PathBuf>,
    #[serde(default = "default_true")]
    use_sgb_colors: bool,
    #[serde(default)]
    window_width: Option<f32>,
    #[serde(default)]
    window_height: Option<f32>,
    #[serde(default)]
    keyboard_bindings: LegacyKeyBindings,
    #[serde(default)]
    gamepad_bindings: LegacyKeyBindings,
}

impl Default for SettingsFile {
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
            library_sort: SortKey::default(),
            library_layout: LibraryLayout::default(),
            window_width: None,
            window_height: None,
            keyboard_controls: Bindings::default_keyboard(),
            gamepad_controls: Bindings::default_gamepad(),
            keyboard_bindings: LegacyActionBindings::default(),
            gamepad_bindings: LegacyActionBindings::default(),
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
    pub library_sort: SortKey,
    pub library_layout: LibraryLayout,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    pub keyboard_bindings: Bindings,
    pub gamepad_bindings: Bindings,
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
            library_sort: SortKey::default(),
            library_layout: LibraryLayout::default(),
            window_width: None,
            window_height: None,
            keyboard_bindings: Bindings::default_keyboard(),
            gamepad_bindings: Bindings::default_gamepad(),
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

        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };

        // Try current format (control-id keyed bindings), migrating any
        // action-name keyed maps a pre-control-id file carries.
        if let Ok(file) = ron::from_str::<SettingsFile>(&data) {
            let keyboard = if file.keyboard_bindings.0.is_empty() {
                file.keyboard_controls
            } else {
                file.keyboard_bindings.into()
            };
            let gamepad = if file.gamepad_bindings.0.is_empty() {
                file.gamepad_controls
            } else {
                file.gamepad_bindings.into()
            };
            return Self {
                setup_complete: file.setup_complete,
                internet_enabled: file.internet_enabled,
                hasheous_enabled: file.hasheous_enabled,
                homebrew_hub_enabled: file.homebrew_hub_enabled,
                palette: parse_palette(&file.palette),
                rom_directories: file.rom_directories,
                use_sgb_colors: file.use_sgb_colors,
                persistence: file.persistence,
                pixel_grid: file.pixel_grid,
                scanlines: file.scanlines,
                cartridge_rw_enabled: file.cartridge_rw_enabled,
                library_sort: file.library_sort,
                library_layout: file.library_layout,
                window_width: file.window_width,
                window_height: file.window_height,
                keyboard_bindings: keyboard,
                gamepad_bindings: gamepad,
            };
        }

        // Try legacy format (flat KeyBindings struct) and migrate
        if let Ok(file) = ron::from_str::<LegacySettingsFile>(&data) {
            let mut keyboard: Bindings = file.keyboard_bindings.into();
            let gamepad: Bindings = file.gamepad_bindings.into();

            // Legacy format had no emulator bindings — add defaults for new actions
            if keyboard.get(Action::Screenshot).is_none() {
                keyboard.set(Action::Screenshot, "F12".to_string());
            }
            if keyboard.get(Action::ToggleFullscreen).is_none() {
                keyboard.set(Action::ToggleFullscreen, "F11".to_string());
            }
            if keyboard.get(Action::Pause).is_none() {
                keyboard.set(Action::Pause, "Space".to_string());
            }

            let settings = Self {
                setup_complete: file.setup_complete,
                internet_enabled: file.internet_enabled,
                hasheous_enabled: true,
                homebrew_hub_enabled: true,
                palette: parse_palette(&file.palette),
                rom_directories: file.rom_directories,
                use_sgb_colors: file.use_sgb_colors,
                persistence: true,
                pixel_grid: true,
                scanlines: true,
                cartridge_rw_enabled: true,
                library_sort: SortKey::default(),
                library_layout: LibraryLayout::default(),
                window_width: file.window_width,
                window_height: file.window_height,
                keyboard_bindings: keyboard,
                gamepad_bindings: gamepad,
            };
            // Re-save in new format so migration only happens once
            settings.save();
            return settings;
        }

        // Migrate from old key=value format
        let mut settings = Self::default();
        for line in data.lines() {
            if let Some(value) = line.strip_prefix("palette=") {
                settings.palette = parse_palette(value);
            }
        }
        settings.save();
        settings
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
            library_sort: self.library_sort,
            library_layout: self.library_layout,
            window_width: self.window_width,
            window_height: self.window_height,
            keyboard_controls: self.keyboard_bindings.clone(),
            gamepad_controls: self.gamepad_bindings.clone(),
            keyboard_bindings: LegacyActionBindings::default(),
            gamepad_bindings: LegacyActionBindings::default(),
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

    #[test]
    fn migrate_legacy_settings() {
        let old_format = r#"(
            setup_complete: true,
            internet_enabled: true,
            palette: "Pocket",
            rom_directories: ["/home/test/roms"],
            use_sgb_colors: false,
            window_width: Some(1920.0),
            window_height: Some(1080.0),
            keyboard_bindings: (
                a: "x",
                b: "z",
                start: "Enter",
                select: "Shift",
                up: "ArrowUp",
                down: "ArrowDown",
                left: "ArrowLeft",
                right: "ArrowRight",
            ),
            gamepad_bindings: (
                a: "South",
                b: "East",
                start: "Start",
                select: "Select",
                up: "DPadUp",
                down: "DPadDown",
                left: "DPadLeft",
                right: "DPadRight",
            ),
        )"#;

        // New format should fail
        assert!(ron::from_str::<SettingsFile>(old_format).is_err());

        // Legacy format should succeed
        let legacy: LegacySettingsFile = ron::from_str(old_format).unwrap();
        assert!(legacy.setup_complete);
        assert!(legacy.internet_enabled);
        assert_eq!(legacy.palette, "Pocket");
        assert_eq!(
            legacy.rom_directories,
            vec![PathBuf::from("/home/test/roms")]
        );
        assert!(!legacy.use_sgb_colors);

        // Bindings migration
        let keyboard: Bindings = legacy.keyboard_bindings.into();
        assert_eq!(keyboard.get(Action::Control(2)), Some("x"));
        assert_eq!(keyboard.get(Action::Control(3)), Some("z"));
        assert_eq!(keyboard.get(Action::Control(0)), Some("Enter"));
        // Emulator actions not present (added by caller)
        assert_eq!(keyboard.get(Action::Screenshot), None);

        let gamepad: Bindings = legacy.gamepad_bindings.into();
        assert_eq!(gamepad.get(Action::Control(2)), Some("South"));
        assert_eq!(gamepad.get(Action::Screenshot), None);
    }

    #[test]
    fn migrate_action_keyed_bindings() {
        let action_format = r#"(
            setup_complete: true,
            internet_enabled: false,
            palette: "Green",
            rom_directories: ["/home/test/roms"],
            keyboard_bindings: ({
                GbA: "q",
                GbStart: "Enter",
                Screenshot: "F12",
            }),
            gamepad_bindings: ({
                GbB: "West",
            }),
        )"#;

        let file: SettingsFile = ron::from_str(action_format).unwrap();
        assert!(file.setup_complete);
        assert_eq!(file.rom_directories, vec![PathBuf::from("/home/test/roms")]);
        let keyboard: Bindings = file.keyboard_bindings.into();
        assert_eq!(keyboard.get(Action::Control(2)), Some("q"));
        assert_eq!(keyboard.get(Action::Control(0)), Some("Enter"));
        assert_eq!(keyboard.get(Action::Screenshot), Some("F12"));
        let gamepad: Bindings = file.gamepad_bindings.into();
        assert_eq!(gamepad.get(Action::Control(3)), Some("West"));
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
    fn parse_new_format() {
        let new_format = r#"(
            setup_complete: true,
            internet_enabled: false,
            palette: "Green",
            rom_directories: [],
            use_sgb_colors: true,
            window_width: Some(1280.0),
            window_height: Some(720.0),
            keyboard_controls: ({
                Control(2): "x",
                Control(3): "z",
                Control(0): "Enter",
                Control(1): "Shift",
                Control(4): "ArrowUp",
                Control(5): "ArrowDown",
                Control(6): "ArrowLeft",
                Control(7): "ArrowRight",
                Screenshot: "F12",
                ToggleFullscreen: "F11",
                Pause: "Space",
            }),
            gamepad_controls: ({
                Control(2): "South",
                Control(3): "East",
            }),
        )"#;

        let file: SettingsFile = ron::from_str(new_format).unwrap();
        assert!(file.setup_complete);
        assert_eq!(file.keyboard_controls.get(Action::Control(2)), Some("x"));
        assert_eq!(file.keyboard_controls.get(Action::Screenshot), Some("F12"));
        assert_eq!(file.gamepad_controls.get(Action::Control(2)), Some("South"));
        assert_eq!(file.gamepad_controls.get(Action::Screenshot), None);
        assert!(file.keyboard_bindings.0.is_empty());
    }
}
