use std::{collections::HashMap, fmt, fs, path::PathBuf};

use missingno_gb::ppu::types::palette::PaletteChoice;
use serde::{Deserialize, Serialize};

// ── Actions ───────────────────────────────────────────────────────────

/// Every bindable action — game buttons and emulator controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    // Game Boy buttons (press/release)
    GbA,
    GbB,
    GbStart,
    GbSelect,
    GbUp,
    GbDown,
    GbLeft,
    GbRight,
    // Emulator actions (fire once on press)
    Screenshot,
    ToggleFullscreen,
    Pause,
}

/// The 8 Game Boy buttons, for iteration and joypad mapping.
pub const GB_ACTIONS: [Action; 8] = [
    Action::GbUp,
    Action::GbDown,
    Action::GbLeft,
    Action::GbRight,
    Action::GbA,
    Action::GbB,
    Action::GbStart,
    Action::GbSelect,
];

/// Emulator-level actions, for iteration.
pub const EMULATOR_ACTIONS: [Action; 3] =
    [Action::Screenshot, Action::ToggleFullscreen, Action::Pause];

impl Action {
    /// True for Game Boy buttons that produce press/release events.
    pub fn is_game_button(self) -> bool {
        matches!(
            self,
            Action::GbA
                | Action::GbB
                | Action::GbStart
                | Action::GbSelect
                | Action::GbUp
                | Action::GbDown
                | Action::GbLeft
                | Action::GbRight
        )
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::GbA => write!(f, "A"),
            Action::GbB => write!(f, "B"),
            Action::GbStart => write!(f, "Start"),
            Action::GbSelect => write!(f, "Select"),
            Action::GbUp => write!(f, "Up"),
            Action::GbDown => write!(f, "Down"),
            Action::GbLeft => write!(f, "Left"),
            Action::GbRight => write!(f, "Right"),
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
            (Action::GbA, "x".to_string()),
            (Action::GbB, "z".to_string()),
            (Action::GbStart, "Enter".to_string()),
            (Action::GbSelect, "Shift".to_string()),
            (Action::GbUp, "ArrowUp".to_string()),
            (Action::GbDown, "ArrowDown".to_string()),
            (Action::GbLeft, "ArrowLeft".to_string()),
            (Action::GbRight, "ArrowRight".to_string()),
            (Action::Screenshot, "F12".to_string()),
            (Action::ToggleFullscreen, "F11".to_string()),
            (Action::Pause, "Space".to_string()),
        ]))
    }

    pub fn default_gamepad() -> Self {
        Self(HashMap::from([
            (Action::GbA, "South".to_string()),
            (Action::GbB, "East".to_string()),
            (Action::GbStart, "Start".to_string()),
            (Action::GbSelect, "Select".to_string()),
            (Action::GbUp, "DPadUp".to_string()),
            (Action::GbDown, "DPadDown".to_string()),
            (Action::GbLeft, "DPadLeft".to_string()),
            (Action::GbRight, "DPadRight".to_string()),
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
#[derive(Deserialize)]
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

impl Default for LegacyKeyBindings {
    fn default() -> Self {
        Self {
            a: String::new(),
            b: String::new(),
            start: String::new(),
            select: String::new(),
            up: String::new(),
            down: String::new(),
            left: String::new(),
            right: String::new(),
        }
    }
}

impl From<LegacyKeyBindings> for Bindings {
    fn from(old: LegacyKeyBindings) -> Self {
        // Only migrate the game button bindings — emulator action defaults
        // are added by the caller based on whether this is keyboard or gamepad.
        let mut map = HashMap::new();
        if !old.a.is_empty() {
            map.insert(Action::GbA, old.a);
        }
        if !old.b.is_empty() {
            map.insert(Action::GbB, old.b);
        }
        if !old.start.is_empty() {
            map.insert(Action::GbStart, old.start);
        }
        if !old.select.is_empty() {
            map.insert(Action::GbSelect, old.select);
        }
        if !old.up.is_empty() {
            map.insert(Action::GbUp, old.up);
        }
        if !old.down.is_empty() {
            map.insert(Action::GbDown, old.down);
        }
        if !old.left.is_empty() {
            map.insert(Action::GbLeft, old.left);
        }
        if !old.right.is_empty() {
            map.insert(Action::GbRight, old.right);
        }
        Bindings(map)
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
    #[serde(default = "default_true")]
    cartridge_rw_enabled: bool,
    #[serde(default)]
    window_width: Option<f32>,
    #[serde(default)]
    window_height: Option<f32>,
    #[serde(default = "default_keyboard_bindings")]
    keyboard_bindings: Bindings,
    #[serde(default = "default_gamepad_bindings")]
    gamepad_bindings: Bindings,
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
            cartridge_rw_enabled: true,
            window_width: None,
            window_height: None,
            keyboard_bindings: Bindings::default_keyboard(),
            gamepad_bindings: Bindings::default_gamepad(),
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
    pub cartridge_rw_enabled: bool,
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
            cartridge_rw_enabled: true,
            window_width: None,
            window_height: None,
            keyboard_bindings: Bindings::default_keyboard(),
            gamepad_bindings: Bindings::default_gamepad(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };

        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };

        // Try current format (HashMap-based bindings)
        if let Ok(file) = ron::from_str::<SettingsFile>(&data) {
            return Self {
                setup_complete: file.setup_complete,
                internet_enabled: file.internet_enabled,
                hasheous_enabled: file.hasheous_enabled,
                homebrew_hub_enabled: file.homebrew_hub_enabled,
                palette: parse_palette(&file.palette),
                rom_directories: file.rom_directories,
                use_sgb_colors: file.use_sgb_colors,
                cartridge_rw_enabled: file.cartridge_rw_enabled,
                window_width: file.window_width,
                window_height: file.window_height,
                keyboard_bindings: file.keyboard_bindings,
                gamepad_bindings: file.gamepad_bindings,
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
                cartridge_rw_enabled: true,
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
            cartridge_rw_enabled: self.cartridge_rw_enabled,
            window_width: self.window_width,
            window_height: self.window_height,
            keyboard_bindings: self.keyboard_bindings.clone(),
            gamepad_bindings: self.gamepad_bindings.clone(),
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
        assert_eq!(keyboard.get(Action::GbA), Some("x"));
        assert_eq!(keyboard.get(Action::GbB), Some("z"));
        assert_eq!(keyboard.get(Action::GbStart), Some("Enter"));
        // Emulator actions not present (added by caller)
        assert_eq!(keyboard.get(Action::Screenshot), None);

        let gamepad: Bindings = legacy.gamepad_bindings.into();
        assert_eq!(gamepad.get(Action::GbA), Some("South"));
        assert_eq!(gamepad.get(Action::Screenshot), None);
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
            keyboard_bindings: ({
                GbA: "x",
                GbB: "z",
                GbStart: "Enter",
                GbSelect: "Shift",
                GbUp: "ArrowUp",
                GbDown: "ArrowDown",
                GbLeft: "ArrowLeft",
                GbRight: "ArrowRight",
                Screenshot: "F12",
                ToggleFullscreen: "F11",
                Pause: "Space",
            }),
            gamepad_bindings: ({
                GbA: "South",
                GbB: "East",
            }),
        )"#;

        let file: SettingsFile = ron::from_str(new_format).unwrap();
        assert!(file.setup_complete);
        assert_eq!(file.keyboard_bindings.get(Action::GbA), Some("x"));
        assert_eq!(file.keyboard_bindings.get(Action::Screenshot), Some("F12"));
        assert_eq!(file.gamepad_bindings.get(Action::GbA), Some("South"));
        assert_eq!(file.gamepad_bindings.get(Action::Screenshot), None);
    }
}
