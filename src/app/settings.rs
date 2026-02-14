use std::{fs, path::PathBuf};

use missingno_core::game_boy::video::palette::PaletteChoice;

#[derive(Default)]
pub struct Settings {
    pub palette: PaletteChoice,
}

impl Settings {
    pub fn load() -> Self {
        let Some(data) = settings_path().and_then(|path| fs::read_to_string(path).ok()) else {
            return Self::default();
        };
        let mut settings = Self::default();
        for line in data.lines() {
            if let Some(value) = line.strip_prefix("palette=") {
                settings.palette = match value {
                    "Green" => PaletteChoice::Green,
                    "Pocket" => PaletteChoice::Pocket,
                    "Classic" => PaletteChoice::Classic,
                    _ => PaletteChoice::default(),
                };
            }
        }
        settings
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let palette = match self.palette {
            PaletteChoice::Green => "Green",
            PaletteChoice::Pocket => "Pocket",
            PaletteChoice::Classic => "Classic",
        };
        let _ = fs::write(path, format!("palette={palette}\n"));
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("settings"))
}
