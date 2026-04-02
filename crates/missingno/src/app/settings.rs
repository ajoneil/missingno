use std::{fs, path::PathBuf};

use missingno_gb::ppu::types::palette::PaletteChoice;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    setup_complete: bool,
    #[serde(default)]
    internet_enabled: bool,
    #[serde(default)]
    palette: String,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            palette: palette_to_string(PaletteChoice::default()),
        }
    }
}

pub struct Settings {
    pub setup_complete: bool,
    pub internet_enabled: bool,
    pub palette: PaletteChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            setup_complete: false,
            internet_enabled: false,
            palette: PaletteChoice::default(),
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
