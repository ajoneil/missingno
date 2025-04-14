mod audio;
mod cpu;
mod instructions;
mod interrupts;
mod panes;
mod video;

use crate::{emulator::GameBoy, ui};
use audio::audio_pane;
use cpu::cpu_pane;
use iced::{
    Element, Task,
    widget::{
        self, container,
        pane_grid::{self, Axis},
    },
};
use instructions::instructions_pane;
use panes::Pane;
use video::video_pane;

#[derive(Debug, Clone)]
pub enum Message {
    Step,
    Run,
    SetBreakpoint(u16),
    ClearBreakpoint(u16),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

pub struct Debugger {
    debugger: crate::debugger::Debugger,
    panes: pane_grid::State<Pane>,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        let (mut panes, instructions_pane) = pane_grid::State::new(Pane::Instructions);
        let (cpu_plane, split) = panes
            .split(Axis::Vertical, instructions_pane, Pane::Cpu)
            .unwrap();
        panes.resize(split, 1.0 / 4.0);
        let (_, split) = panes.split(Axis::Vertical, cpu_plane, Pane::Video).unwrap();
        panes.resize(split, 1.0 / 3.0);
        panes.split(Axis::Horizontal, cpu_plane, Pane::Audio);

        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }
}

pub fn update(debugger: &mut Debugger, message: Message) -> Task<ui::Message> {
    let Debugger { panes, debugger } = debugger;

    match message {
        Message::Step => debugger.step(),
        Message::Run => debugger.run(),
        Message::SetBreakpoint(address) => debugger.set_breakpoint(address),
        Message::ClearBreakpoint(address) => debugger.clear_breakpoint(address),

        Message::ResizePane(resize) => panes.resize(resize.split, resize.ratio),
        Message::DragPane(drag) => match drag {
            pane_grid::DragEvent::Dropped { pane, target } => panes.drop(pane, target),
            _ => {}
        },
    }

    Task::none()
}

pub fn debugger(debugger: &Debugger) -> Element<'_, ui::Message> {
    container(
        widget::pane_grid(&debugger.panes, |_pane, state, _is_maximized| match state {
            Pane::Instructions => instructions_pane(
                debugger.game_boy().cartridge(),
                debugger.game_boy().cpu().program_counter,
                debugger.debugger.breakpoints(),
            ),
            Pane::Cpu => cpu_pane(&debugger.debugger),
            Pane::Video => video_pane(debugger.game_boy().video()),
            Pane::Audio => audio_pane(debugger.game_boy().audio()),
        })
        .on_resize(10.0, |resize| Message::ResizePane(resize).into())
        .on_drag(|drag| Message::DragPane(drag).into())
        .spacing(10),
    )
    .padding(10)
    .into()
}
