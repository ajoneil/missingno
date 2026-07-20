use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Padding,
    widget::{Stack, center, column, container, mouse_area, opaque, row, svg, text as iced_text},
};

use super::friendly_ago;
use crate::app::ui::{
    buttons, containers, fonts,
    icons::{self, Icon},
    menu_divider,
    sizes::{border_m, l, m, s},
    text,
};
use crate::app::{
    App, DetailMessage, DetailSubScreen, Message, PendingAction, Screen, debugger, load,
};

impl App {
    pub(super) fn apply_toast<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        let mut layers = vec![content];
        if self.screenshot_toast.is_some() {
            layers.push(screenshot_toast());
        }
        if let Some((message, _)) = &self.notice {
            layers.push(notice_toast(message));
        }
        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        }
    }

    pub(super) fn apply_menu<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        if !self.menu_open {
            return content;
        }

        let mut items = column![].spacing(2).width(220);
        let mut has_items = false;

        // Per-screen menu items
        match &self.screen {
            Screen::Library { .. } | Screen::HomebrewBrowser { .. } => {
                items = items.push(menu_item(
                    Icon::FolderOpen,
                    "Open ROM file...",
                    load::Message::Pick.into(),
                ));
                has_items = true;
            }
            Screen::ViewingGame {
                sub_screen: DetailSubScreen::Detail { .. },
                ..
            } => {
                items = items.push(menu_item(
                    Icon::FolderOpen,
                    "Open ROM file...",
                    load::Message::Pick.into(),
                ));
                items = items.push(menu_divider());
                items = items.push(menu_item(
                    Icon::Download,
                    "Import Save...",
                    Message::Detail(DetailMessage::ImportSave),
                ));
                items = items.push(menu_item(
                    Icon::FolderOpen,
                    "Open Folder",
                    Message::Detail(DetailMessage::OpenGameFolder),
                ));
                items = items.push(menu_item(
                    Icon::Globe,
                    "Refresh Metadata",
                    Message::Detail(DetailMessage::RefreshMetadata),
                ));
                items = items.push(menu_divider());
                items = items.push(menu_item_danger(
                    Icon::Close,
                    "Remove Game",
                    Message::Detail(DetailMessage::RemoveGame),
                ));
                has_items = true;
            }
            Screen::Emulator => {
                if !self.debugger_enabled {
                    items = items.push(menu_item(
                        Icon::Debug,
                        "Debugger",
                        Message::ToggleDebugger(true),
                    ));
                    items = items.push(menu_divider());
                }
                if self.debugger_enabled {
                    items = items.push(menu_item(
                        Icon::Play,
                        "Step Frame",
                        debugger::Message::StepFrame.into(),
                    ));
                }
                items = items.push(menu_item_danger(Icon::Close, "Reset", Message::Reset));
                items = items.push(menu_divider());
                items = items.push(menu_item(
                    Icon::Camera,
                    "Screenshot",
                    Message::TakeScreenshot,
                ));
                if self.debugger_enabled {
                    items = items.push(menu_item(
                        Icon::Download,
                        "Capture Trace",
                        debugger::Message::CaptureFrame.into(),
                    ));
                }
                has_items = true;
            }
            _ => {}
        }

        // Settings always last
        if has_items {
            items = items.push(menu_divider());
        }
        items = items.push(menu_item(Icon::Gear, "Settings", Message::ShowSettings));

        let menu_panel = container(items.padding(s())).style(containers::menu);

        // Anchor top-right: scrim covers everything, menu sits in corner
        Stack::new()
            .push(content)
            .push(opaque(
                mouse_area(container(menu_panel).align_right(Fill).padding(Padding {
                    top: m() + 40.0,
                    right: m(),
                    bottom: 0.0,
                    left: 0.0,
                }))
                .on_press(Message::DismissMenu),
            ))
            .into()
    }

    pub(super) fn apply_confirmation_dialog<'a>(
        &self,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let Some(action) = &self.pending_action else {
            return content;
        };

        let (prompt, confirm_label) = match action {
            PendingAction::SwitchGame(_) => ("Close the current game and switch?", "Close Game"),
            PendingAction::CloseApp => ("Close the current game and quit?", "Quit"),
            PendingAction::ResetEmulator => (
                "Reset the emulator? Unsaved progress will be lost.",
                "Reset",
            ),
            PendingAction::StopGame => ("Stop playing and end this session?", "Stop"),
            PendingAction::RemoveGameFromLibrary => {
                ("Remove this game and all its save data?", "Remove")
            }
        };

        let mut info = column![iced_text(prompt)].spacing(s());

        if let Some(current) = &self.current_game {
            info = info.push(
                iced_text(current.entry.display_title())
                    .size(text::sizes::xl())
                    .font(fonts::heading()),
            );
            let last_save_time = current.session.as_ref().and_then(|s| s.last_save_time());
            if let Some(ts) = last_save_time {
                info = info.push(
                    iced_text(format!("Last saved {}", friendly_ago(ts)))
                        .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)),
                );
            } else {
                info = info
                    .push(iced_text("No saves").color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.6)));
            }
        }

        Stack::new()
            .push(content)
            .push(opaque(
                mouse_area(
                    center(
                        container(
                            column![
                                info,
                                row![
                                    buttons::standard("Cancel").on_press(Message::DismissConfirm),
                                    buttons::danger(confirm_label).on_press(Message::ConfirmAction),
                                ]
                                .spacing(s()),
                            ]
                            .spacing(l())
                            .align_x(Center),
                        )
                        .padding(l())
                        .style(containers::menu),
                    )
                    .style(|_| container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                        ..Default::default()
                    }),
                )
                .on_press(Message::DismissConfirm),
            ))
            .into()
    }
}

fn menu_item<'a>(icon: Icon, label: &'a str, message: Message) -> Element<'a, Message> {
    buttons::subtle(row![icons::m(icon), label].spacing(s()).align_y(Center))
        .on_press(Message::MenuAction(Box::new(message)))
        .width(Fill)
        .into()
}

fn menu_item_danger<'a>(icon: Icon, label: &'a str, message: Message) -> Element<'a, Message> {
    buttons::danger(row![icons::m(icon), label].spacing(s()).align_y(Center))
        .on_press(Message::MenuAction(Box::new(message)))
        .width(Fill)
        .into()
}

fn screenshot_toast<'a>() -> Element<'a, Message> {
    container(
        container(
            row![
                icons::m(Icon::Camera).style(|_, _| svg::Style {
                    color: Some(iced::Color::WHITE),
                }),
                iced_text("Screenshot saved").color(iced::Color::WHITE),
            ]
            .spacing(s())
            .align_y(Center),
        )
        .padding(s())
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
            border: iced::Border::default().rounded(border_m()),
            ..Default::default()
        }),
    )
    .align_bottom(Fill)
    .align_right(Fill)
    .padding(l())
    .into()
}

/// A transient status-line toast (save/load result, recording lifecycle,
/// replay divergence). Sits above the screenshot toast's corner so the two
/// don't overlap when both are up.
fn notice_toast<'a>(message: &str) -> Element<'a, Message> {
    container(
        container(iced_text(message.to_owned()).color(iced::Color::WHITE))
            .padding(s())
            .style(|_| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                border: iced::Border::default().rounded(border_m()),
                ..Default::default()
            }),
    )
    .align_bottom(Fill)
    .align_left(Fill)
    .padding(l())
    .into()
}
