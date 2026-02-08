use std::{
    fs,
    path::{Path, PathBuf},
};

use iced::{
    Alignment::Center,
    Element,
    widget::{Column, column, row, text},
};
use serde::{Deserialize, Serialize};

use crate::app::{
    self,
    core::{
        buttons,
        sizes::{m, s},
        text as app_text,
    },
    load,
};

const MAX_RECENT: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
struct RecentGame {
    path: PathBuf,
    title: String,
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
        let _ = ron::to_string(&self.games)
            .ok()
            .map(|data| fs::write(path, data));
    }

    pub fn add(&mut self, path: PathBuf, title: String) {
        self.games.retain(|g| g.path != path);
        self.games.insert(0, RecentGame { path, title });
        self.games.truncate(MAX_RECENT);
    }

    pub fn remove(&mut self, path: &Path) {
        self.games.retain(|g| g.path != path);
    }

    pub fn most_recent_dir(&self) -> Option<PathBuf> {
        self.games
            .first()
            .and_then(|g| g.path.parent().map(Path::to_path_buf))
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let heading = app_text::m("Recent Games");

        let entries = Column::with_children(self.games.iter().map(|game| {
            let label = row![
                text(game.title.clone()),
                text(game.path.display().to_string())
                    .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.4))
                    .size(12.0),
            ]
            .spacing(s())
            .align_y(Center);

            buttons::text(label)
                .on_press(load::Message::LoadPath(game.path.clone()).into())
                .into()
        }))
        .spacing(0);

        column![heading, entries].spacing(m()).into()
    }
}

fn recent_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("recent.ron"))
}
