use std::{
    fs,
    path::{Path, PathBuf},
};

use iced::{
    Alignment::Center,
    Element,
    widget::{Column, column, row, text},
};
use nanoserde::{DeRon, SerRon};

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

#[derive(SerRon, DeRon, Clone)]
struct RecentGame {
    path: String,
    title: String,
}

impl RecentGame {
    fn path(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

pub struct RecentGames {
    games: Vec<RecentGame>,
}

impl RecentGames {
    pub fn load() -> Self {
        let games = recent_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|data| Vec::<RecentGame>::deserialize_ron(&data).ok())
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
        let _ = fs::write(path, self.games.serialize_ron());
    }

    pub fn add(&mut self, path: PathBuf, title: String) {
        let path_str = path.to_string_lossy().into_owned();
        self.games.retain(|g| g.path != path_str);
        self.games.insert(
            0,
            RecentGame {
                path: path_str,
                title,
            },
        );
        self.games.truncate(MAX_RECENT);
    }

    pub fn remove(&mut self, path: &Path) {
        let path_str = path.to_string_lossy();
        self.games.retain(|g| g.path != *path_str);
    }

    pub fn most_recent_dir(&self) -> Option<PathBuf> {
        self.games
            .first()
            .and_then(|g| g.path().parent().map(Path::to_path_buf))
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let heading = app_text::m("Recent Games");

        let entries = Column::with_children(self.games.iter().map(|game| {
            let label = row![
                text(game.title.clone()),
                text(game.path.clone())
                    .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.4))
                    .size(12.0),
            ]
            .spacing(s())
            .align_y(Center);

            buttons::text(label)
                .on_press(load::Message::LoadPath(game.path()).into())
                .into()
        }))
        .spacing(0);

        column![heading, entries].spacing(m()).into()
    }
}

fn recent_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("missingno").join("recent.ron"))
}
