use crate::emulation::{Cartridge, GameBoy};
use iced::{
    Element,
    Length::Fill,
    Task, Theme,
    widget::{button, container, text},
};
use rfd::{AsyncFileDialog, FileHandle};

pub fn run() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .theme(theme)
        .run()
}

fn theme(_app: &App) -> Theme {
    Theme::Dark
}

#[derive(Default)]
struct App {
    load_state: LoadState,
}

#[derive(Default)]
enum LoadState {
    #[default]
    Unloaded,
    Loading,
    Loaded(GameBoy),
}

#[derive(Debug, Clone)]
enum Message {
    PickGameRom,
    GameRomPicked(Option<FileHandle>),
    GameRomLoaded(Vec<u8>),
}

impl App {
    fn title(&self) -> String {
        if let LoadState::Loaded(game_boy) = &self.load_state {
            format!("{} - MissingNo.", game_boy.cartridge().title())
        } else {
            "MissingNo.".into()
        }
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PickGameRom => {
                self.load_state = LoadState::Loading;
                Task::perform(
                    AsyncFileDialog::new()
                        .add_filter("Game Boy ROM", &["gb"])
                        .pick_file(),
                    Message::GameRomPicked,
                )
            }
            Message::GameRomPicked(file_handle) => {
                if let Some(handle) = file_handle {
                    let file = handle.clone();
                    Task::perform(async move { file.read().await }, Message::GameRomLoaded)
                } else {
                    self.load_state = LoadState::Unloaded;
                    Task::none()
                }
            }
            Message::GameRomLoaded(rom) => {
                self.load_state = LoadState::Loaded(GameBoy::new(Cartridge::new(rom)));
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        container(self.inner()).center_x(Fill).center_y(Fill).into()
    }

    fn inner(&self) -> Element<'_, Message> {
        match &self.load_state {
            LoadState::Unloaded => button("Load game").on_press(Message::PickGameRom).into(),
            LoadState::Loading => button("Load game").into(),
            LoadState::Loaded(game_boy) => Self::loaded_view(&game_boy),
        }
    }

    fn loaded_view(game_boy: &GameBoy) -> Element<'_, Message> {
        text(game_boy.cartridge().title()).into()
    }
}
