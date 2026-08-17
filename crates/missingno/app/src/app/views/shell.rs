use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Padding,
    widget::{
        Button, Row, Stack, center, column, container, mouse_area, opaque, row, svg,
        text as iced_text,
    },
};

use super::friendly_ago;
use crate::app::automation::ids;
use crate::app::ui::{
    buttons, containers, fonts,
    icons::{self, Icon},
    menu_divider,
    sizes::{border_m, l, m, s},
    text,
};
use crate::app::{
    App, DetailMessage, DetailSubScreen, Message, Notice, PendingAction, Screen, debugger, launch,
    load,
};

impl App {
    /// The launch window stands over whatever screen raised it, so every entry
    /// point reaches the same one.
    pub(super) fn apply_launch_window<'a>(
        &self,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        match &self.launch_window {
            Some(window) => Stack::new()
                .push(content)
                .push(launch::window(window))
                .into(),
            None => content,
        }
    }

    pub(super) fn apply_toast<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        match &self.notice {
            Some((notice, _)) => Stack::new().push(content).push(notice_toast(notice)).into(),
            None => content,
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
                    ids::MENU_OPEN_ROM,
                    Icon::FolderOpen,
                    "Open ROM file...",
                    load::Message::Pick.into(),
                    buttons::subtle,
                ));
                has_items = true;
            }
            Screen::ViewingGame {
                sub_screen: DetailSubScreen::Detail { .. },
                ..
            } => {
                items = items.push(menu_item(
                    ids::MENU_OPEN_ROM,
                    Icon::FolderOpen,
                    "Open ROM file...",
                    load::Message::Pick.into(),
                    buttons::subtle,
                ));
                items = items.push(menu_divider());
                items = items.push(menu_item(
                    ids::MENU_IMPORT_SAVE,
                    Icon::Download,
                    "Import Save...",
                    Message::Detail(DetailMessage::ImportSave),
                    buttons::subtle,
                ));
                items = items.push(menu_item(
                    ids::MENU_OPEN_FOLDER,
                    Icon::FolderOpen,
                    "Open Folder",
                    Message::Detail(DetailMessage::OpenGameFolder),
                    buttons::subtle,
                ));
                items = items.push(menu_item(
                    ids::MENU_REFRESH_METADATA,
                    Icon::Globe,
                    "Refresh Metadata",
                    Message::Detail(DetailMessage::RefreshMetadata),
                    buttons::subtle,
                ));
                items = items.push(menu_divider());
                items = items.push(menu_item(
                    ids::MENU_REMOVE_GAME,
                    Icon::Trash,
                    "Remove Game",
                    Message::Detail(DetailMessage::RemoveGame),
                    buttons::danger,
                ));
                has_items = true;
            }
            Screen::Emulator => {
                if !self.debugger_enabled {
                    items = items.push(menu_item(
                        ids::MENU_DEBUGGER,
                        Icon::Debug,
                        "Debugger",
                        Message::ToggleDebugger(true),
                        buttons::subtle,
                    ));
                    items = items.push(menu_divider());
                }
                if self.debugger_enabled {
                    items = items.push(menu_item(
                        ids::MENU_STEP_FRAME,
                        Icon::Forward,
                        "Step Frame",
                        debugger::Message::StepFrame.into(),
                        buttons::subtle,
                    ));
                }
                items = items.push(menu_item(
                    ids::MENU_RESET,
                    Icon::Reload,
                    "Reset",
                    Message::Reset,
                    buttons::danger,
                ));
                items = items.push(menu_divider());
                items = items.push(menu_item(
                    ids::MENU_SCREENSHOT,
                    Icon::Camera,
                    "Screenshot",
                    Message::TakeScreenshot,
                    buttons::subtle,
                ));
                if self.debugger_enabled {
                    items = items.push(menu_item(
                        ids::MENU_CAPTURE_TRACE,
                        Icon::Download,
                        "Capture Trace",
                        debugger::Message::CaptureFrame.into(),
                        buttons::subtle,
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
        items = items.push(menu_item(
            ids::MENU_SETTINGS,
            Icon::Gear,
            "Settings",
            Message::ShowSettings,
            buttons::subtle,
        ));

        let menu_panel = container(items.padding(s())).style(containers::menu);

        // Anchor top-right: scrim covers everything, menu sits in corner. The
        // scrim carries the dismiss id so automation can close the menu.
        let scrim = container(opaque(
            mouse_area(container(menu_panel).align_right(Fill).padding(Padding {
                top: m() + 40.0,
                right: m(),
                bottom: 0.0,
                left: 0.0,
            }))
            .on_press(Message::DismissMenu),
        ))
        .id(iced::widget::Id::from(ids::MENU_DISMISS.to_string()))
        .width(Fill)
        .height(Fill);

        Stack::new().push(content).push(scrim).into()
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
                                    crate::app::automation::tag(
                                        ids::CONFIRM_CANCEL,
                                        buttons::standard("Cancel")
                                            .on_press(Message::DismissConfirm),
                                    ),
                                    crate::app::automation::tag(
                                        ids::CONFIRM_ACCEPT,
                                        buttons::danger(confirm_label)
                                            .on_press(Message::ConfirmAction),
                                    ),
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

fn menu_item<'a>(
    id: &str,
    icon: Icon,
    label: &'a str,
    message: Message,
    style: fn(Row<'a, Message>) -> Button<'a, Message>,
) -> Element<'a, Message> {
    tagged_full_width(
        id,
        style(row![icons::m(icon), label].spacing(s()).align_y(Center))
            .on_press(Message::MenuAction(Box::new(message)))
            .width(Fill),
    )
}

/// Wrap a full-width menu control in an automation-tagged container that keeps
/// its width, so its on-screen bounds resolve for the automation surface.
fn tagged_full_width<'a>(
    id: &str,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .id(iced::widget::Id::from(id.to_string()))
        .width(Fill)
        .into()
}

/// The one transient toast — screenshot, save/load outcome, recording
/// lifecycle, replay divergence. Bottom right, above the picture.
fn notice_toast<'a>(notice: &Notice) -> Element<'a, Message> {
    let mut line = row![].spacing(s()).align_y(Center);
    if let Some(icon) = notice.icon {
        line = line.push(icons::m(icon).style(|_, _| svg::Style {
            color: Some(iced::Color::WHITE),
        }));
    }
    line = line.push(iced_text(notice.message.clone()).color(iced::Color::WHITE));
    if let Some(action) = &notice.action {
        line = line.push(buttons::standard(action.label).on_press((*action.message).clone()));
        line = line.push(buttons::subtle("Dismiss").on_press(Message::DismissNotice));
    }

    container(container(line).padding(s()).style(|_| container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
        border: iced::Border::default().rounded(border_m()),
        ..Default::default()
    }))
    .align_bottom(Fill)
    .align_right(Fill)
    .padding(l())
    .into()
}
