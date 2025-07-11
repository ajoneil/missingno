use iced::Element;

use crate::app::Message;

pub mod buttons;
pub mod fonts;
pub mod icons;
pub mod sizes;
pub mod text;

pub fn horizontal_rule() -> Element<'static, Message> {
    iced::widget::horizontal_rule(1).into()
}
