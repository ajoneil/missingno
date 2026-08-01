use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
struct RecentGame {
    sha1: String,
    title: String,
    rom_path: PathBuf,
}

pub struct RecentGames {
    games: Vec<RecentGame>,
}

impl RecentGames {
    pub fn load() -> Self {
        let games = recent_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|data| ron::from_str::<Vec<RecentGame>>(&data).ok())
            .unwrap_or_default();

        Self { games }
    }

    pub fn save(&self) {
        let Some(path) = recent_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(data) = ron::ser::to_string_pretty(&self.games, ron::ser::PrettyConfig::default())
        {
            let _ = fs::write(path, data);
        }
    }

    pub fn add(&mut self, sha1: &str, title: &str, rom_path: &Path) {
        self.games.retain(|g| g.sha1 != sha1);
        self.games.insert(
            0,
            RecentGame {
                sha1: sha1.to_string(),
                title: title.to_string(),
                rom_path: rom_path.to_path_buf(),
            },
        );
        self.games.truncate(MAX_RECENT);
    }

    pub fn update_title(&mut self, sha1: &str, title: &str) {
        for game in &mut self.games {
            if game.sha1 == sha1 {
                game.title = title.to_string();
            }
        }
    }

    pub fn remove_path(&mut self, path: &Path) {
        let path_str = path.to_string_lossy();
        self.games
            .retain(|g| g.rom_path.to_string_lossy() != path_str);
    }

    pub fn most_recent_dir(&self) -> Option<PathBuf> {
        self.games
            .first()
            .and_then(|g| g.rom_path.parent().map(Path::to_path_buf))
    }
}

fn recent_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("recent.ron"))
}
