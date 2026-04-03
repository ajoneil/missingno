use std::{fmt, fs, path::PathBuf};

use missingno_gb::ppu::types::palette::PaletteChoice;
use serde::{Deserialize, Serialize};

/// The 8 Game Boy buttons, as a flat enum for keybinding configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GbButton {
    A,
    B,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
}

impl GbButton {
    pub const ALL: [GbButton; 8] = [
        GbButton::Up,
        GbButton::Down,
        GbButton::Left,
        GbButton::Right,
        GbButton::A,
        GbButton::B,
        GbButton::Start,
        GbButton::Select,
    ];
}

impl fmt::Display for GbButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GbButton::A => write!(f, "A"),
            GbButton::B => write!(f, "B"),
            GbButton::Start => write!(f, "Start"),
            GbButton::Select => write!(f, "Select"),
            GbButton::Up => write!(f, "Up"),
            GbButton::Down => write!(f, "Down"),
            GbButton::Left => write!(f, "Left"),
            GbButton::Right => write!(f, "Right"),
        }
    }
}

/// Serializable keybinding map: one string per Game Boy button.
#[derive(Serialize, Deserialize, Clone, Hash)]
pub struct KeyBindings {
    pub a: String,
    pub b: String,
    pub start: String,
    pub select: String,
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
}

impl KeyBindings {
    pub fn get(&self, button: GbButton) -> &str {
        match button {
            GbButton::A => &self.a,
            GbButton::B => &self.b,
            GbButton::Start => &self.start,
            GbButton::Select => &self.select,
            GbButton::Up => &self.up,
            GbButton::Down => &self.down,
            GbButton::Left => &self.left,
            GbButton::Right => &self.right,
        }
    }

    pub fn set(&mut self, button: GbButton, value: String) {
        match button {
            GbButton::A => self.a = value,
            GbButton::B => self.b = value,
            GbButton::Start => self.start = value,
            GbButton::Select => self.select = value,
            GbButton::Up => self.up = value,
            GbButton::Down => self.down = value,
            GbButton::Left => self.left = value,
            GbButton::Right => self.right = value,
        }
    }
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self::default_keyboard()
    }
}

impl KeyBindings {
    pub const DEFAULT_KEYBOARD: Self = Self {
        a: String::new(), b: String::new(), start: String::new(), select: String::new(),
        up: String::new(), down: String::new(), left: String::new(), right: String::new(),
    };

    pub const DEFAULT_GAMEPAD: Self = Self {
        a: String::new(), b: String::new(), start: String::new(), select: String::new(),
        up: String::new(), down: String::new(), left: String::new(), right: String::new(),
    };

    pub fn default_keyboard() -> Self {
        Self {
            a: "x".to_string(),
            b: "z".to_string(),
            start: "Enter".to_string(),
            select: "Shift".to_string(),
            up: "ArrowUp".to_string(),
            down: "ArrowDown".to_string(),
            left: "ArrowLeft".to_string(),
            right: "ArrowRight".to_string(),
        }
    }

    pub fn default_gamepad() -> Self {
        Self {
            a: "South".to_string(),
            b: "East".to_string(),
            start: "Start".to_string(),
            select: "Select".to_string(),
            up: "DPadUp".to_string(),
            down: "DPadDown".to_string(),
            left: "DPadLeft".to_string(),
            right: "DPadRight".to_string(),
        }
    }
}

fn default_keyboard_bindings() -> KeyBindings { KeyBindings::default_keyboard() }
fn default_gamepad_bindings() -> KeyBindings { KeyBindings::default_gamepad() }

#[derive(Serialize, Deserialize)]
struct SettingsFile {
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
    #[serde(default = "default_keyboard_bindings")]
    keyboard_bindings: KeyBindings,
    #[serde(default = "default_gamepad_bindings")]
    gamepad_bindings: KeyBindings,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            palette: palette_to_string(PaletteChoice::default()),
            rom_directories: Vec::new(),
            use_sgb_colors: true,
            window_width: None,
            window_height: None,
            keyboard_bindings: KeyBindings::default_keyboard(),
            gamepad_bindings: KeyBindings::default_gamepad(),
        }
    }
}

fn default_true() -> bool { true }

pub struct Settings {
    pub setup_complete: bool,
    pub internet_enabled: bool,
    pub palette: PaletteChoice,
    pub rom_directories: Vec<PathBuf>,
    pub use_sgb_colors: bool,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    pub keyboard_bindings: KeyBindings,
    pub gamepad_bindings: KeyBindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            palette: PaletteChoice::default(),
            rom_directories: Vec::new(),
            use_sgb_colors: true,
            window_width: None,
            window_height: None,
            keyboard_bindings: KeyBindings::default_keyboard(),
            gamepad_bindings: KeyBindings::default_gamepad(),
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

        // Try RON format
        if let Ok(file) = ron::from_str::<SettingsFile>(&data) {
            return Self {
                setup_complete: file.setup_complete,
                internet_enabled: file.internet_enabled,
                palette: parse_palette(&file.palette),
                rom_directories: file.rom_directories,
                use_sgb_colors: file.use_sgb_colors,
                window_width: file.window_width,
                window_height: file.window_height,
                keyboard_bindings: file.keyboard_bindings,
                gamepad_bindings: file.gamepad_bindings,
            };
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
            palette: palette_to_string(self.palette),
            rom_directories: self.rom_directories.clone(),
            use_sgb_colors: self.use_sgb_colors,
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
