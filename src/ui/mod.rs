use iced::{Element, widget::text};

pub fn run() -> iced::Result {
    iced::application("MissingNo.", App::update, App::view).run()
}

#[derive(Default)]
struct App {}

#[derive(Debug)]
enum Message {}

impl App {
    fn update(&mut self, _message: Message) {}

    fn view(&self) -> Element<'_, Message> {
        text("Hello world").into()
    }
}
