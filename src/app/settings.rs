use std::{fs, path::PathBuf};

use nanoserde::{DeRon, SerRon};

use crate::game_boy::video::palette::PaletteChoice;

#[derive(SerRon, DeRon, Default)]
pub struct Settings {
    pub palette: PaletteChoice,
}

impl Settings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|data| Self::deserialize_ron(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(path, self.serialize_ron());
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("settings.ron"))
}
