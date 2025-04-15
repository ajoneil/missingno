mod audio;
mod breakpoints;
mod cpu;
mod instructions;
mod interrupts;
mod panes;
mod video;

use crate::{emulator::GameBoy, ui};
use audio::audio_pane;
use breakpoints::breakpoints_pane;
use cpu::cpu_pane;
use iced::{
    Element, Task,
    widget::{
        self, container,
        pane_grid::{self, Axis},
    },
};
use instructions::instructions_pane;
use panes::PaneState;
use video::video_pane;

use super::styles::spacing;

#[derive(Debug, Clone)]
pub enum Message {
    Step,
    StepOver,
    StepFrame,
    Run,
    SetBreakpoint(u16),
    ClearBreakpoint(u16),

    BreakpointPane(breakpoints::Message),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

pub struct State {
    debugger: crate::debugger::Debugger,
    panes: pane_grid::State<PaneState>,
    breakpoint_pane: pane_grid::Pane,
}

impl State {
    pub fn new(game_boy: GameBoy) -> Self {
        let (mut panes, instructions_pane) = pane_grid::State::new(PaneState::Instructions);
        let (cpu_plane, split) = panes
            .split(Axis::Vertical, instructions_pane, PaneState::Cpu)
            .unwrap();
        panes.resize(split, 1.0 / 4.0);

        let (breakpoint_pane, split) = panes
            .split(
                Axis::Horizontal,
                instructions_pane,
                PaneState::Breakpoints(breakpoints::State::new()),
            )
            .unwrap();
        panes.resize(split, 3.0 / 4.0);

        let (_, split) = panes
            .split(Axis::Vertical, cpu_plane, PaneState::Video)
            .unwrap();
        panes.resize(split, 1.0 / 3.0);
        panes.split(Axis::Horizontal, cpu_plane, PaneState::Audio);

        Self {
            debugger: crate::debugger::Debugger::new(game_boy),
            panes,
            breakpoint_pane,
        }
    }

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }
}

pub fn update(state: &mut State, message: Message) -> Task<ui::Message> {
    let State {
        panes, debugger, ..
    } = state;

    match message {
        Message::Step => debugger.step(),
        Message::StepOver => debugger.step_over(),
        Message::StepFrame => debugger.step_frame(),
        Message::Run => debugger.run(),
        Message::SetBreakpoint(address) => debugger.set_breakpoint(address),
        Message::ClearBreakpoint(address) => debugger.clear_breakpoint(address),

        Message::BreakpointPane(message) => {
            if let PaneState::Breakpoints(breakpoints_state) =
                panes.get_mut(state.breakpoint_pane).unwrap()
            {
                breakpoints_state.update(message, debugger);
            }
        }

        Message::ResizePane(resize) => panes.resize(resize.split, resize.ratio),
        Message::DragPane(drag) => match drag {
            pane_grid::DragEvent::Dropped { pane, target } => panes.drop(pane, target),
            _ => {}
        },
    }

    Task::none()
}

pub fn debugger(state: &State) -> Element<'_, ui::Message> {
    container(
        widget::pane_grid(
            &state.panes,
            |_pane, pane_state, _is_maximized| match pane_state {
                PaneState::Instructions => instructions_pane(
                    state.game_boy().memory_mapped(),
                    state.game_boy().cpu().program_counter,
                    state.debugger.breakpoints(),
                ),
                PaneState::Breakpoints(breakpoint_state) => {
                    breakpoints_pane(&state.debugger, breakpoint_state)
                }
                PaneState::Cpu => cpu_pane(&state.debugger),
                PaneState::Video => video_pane(state.game_boy().video()),
                PaneState::Audio => audio_pane(state.game_boy().audio()),
            },
        )
        .on_resize(10.0, |resize| Message::ResizePane(resize).into())
        .on_drag(|drag| Message::DragPane(drag).into())
        .spacing(spacing::m()),
    )
    .padding(spacing::m())
    .into()
}
