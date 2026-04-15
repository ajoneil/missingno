use iced::{
    Border, Color, Element,
    Length::{self, Fill},
    alignment::Vertical,
    widget::{Space, button, column, container, row, rule, text, tooltip},
};

use crate::app::{
    self, debugger,
    ui::{
        fonts, palette,
        sizes::{s, xs},
    },
};
use missingno_gb::cpu::{
    Cpu, HaltState,
    flags::Flags,
    registers::{Register8, Register16},
};
use missingno_gb::debugger::Debugger;
use missingno_gb::ppu::types::palette::Palette;

use super::interrupts::interrupts;
use super::ppu::ppu_sidebar;

/// Monospace text size for all register labels and values.
const REG: f32 = 14.0;
/// Detail text size for collapsed summaries.
const DETAIL: f32 = 11.0;

const SIDEBAR_WIDTH: f32 = 260.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Cpu,
    Ppu,
}

pub struct Sidebar {
    collapsed: [bool; 2], // indexed by Section
}

impl Section {
    fn index(self) -> usize {
        match self {
            Section::Cpu => 0,
            Section::Ppu => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleSection(Section),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Debugger(debugger::Message::Sidebar(message))
    }
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            collapsed: [false, false], // CPU and PPU expanded by default
        }
    }

    fn is_collapsed(&self, section: Section) -> bool {
        self.collapsed[section.index()]
    }

    pub fn update(&mut self, message: &Message) {
        match message {
            Message::ToggleSection(section) => {
                let idx = section.index();
                self.collapsed[idx] = !self.collapsed[idx];
            }
        }
    }

    pub fn view<'a>(
        &'a self,
        debugger: &'a Debugger,
        pal: &'a Palette,
    ) -> Element<'a, app::Message> {
        let game_boy = debugger.game_boy();

        column![
            self.cpu_section(game_boy.cpu(), game_boy),
            self.ppu_section(game_boy.ppu(), pal),
        ]
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .height(Fill)
        .spacing(s())
        .into()
    }

    fn cpu_section<'a>(
        &self,
        cpu: &'a Cpu,
        game_boy: &'a missingno_gb::GameBoy,
    ) -> Element<'a, app::Message> {
        let summary = format!("pc {:04X} · sp {:04X}", cpu.bus_counter, cpu.stack_pointer,);
        let collapsed = self.is_collapsed(Section::Cpu);

        let body = column![
            pointers(cpu),
            rule::horizontal(1),
            register_a_row(cpu),
            register_pair_row(cpu, Register8::B, Register8::C, Register16::Bc),
            register_pair_row(cpu, Register8::D, Register8::E, Register16::De),
            register_pair_row(cpu, Register8::H, Register8::L, Register16::Hl),
            rule::horizontal(1),
            interrupts(game_boy),
        ]
        .padding(s())
        .spacing(s())
        .into();

        let running = cpu.halt_state != HaltState::Halted;
        section(
            "CPU",
            &summary,
            collapsed,
            Section::Cpu,
            Some((running, palette::GREEN)),
            None,
            body,
        )
    }

    fn ppu_section<'a>(
        &self,
        ppu: &'a missingno_gb::ppu::Ppu,
        pal: &'a Palette,
    ) -> Element<'a, app::Message> {
        let mode = ppu.mode();
        let (mode_text, mode_color) = mode_display(mode);
        let summary = format!("{} · ly {}", mode_text, ppu.video.ly());
        let collapsed = self.is_collapsed(Section::Ppu);

        let mode_detail: Element<'_, app::Message> = text(mode_text)
            .font(fonts::monospace())
            .size(DETAIL)
            .color(mode_color)
            .into();

        section(
            "PPU",
            &summary,
            collapsed,
            Section::Ppu,
            Some((ppu.control().video_enabled(), palette::GREEN)),
            Some(mode_detail),
            ppu_sidebar(ppu, pal),
        )
    }
}

// --- Collapsible section ---

fn section<'a>(
    label: &'a str,
    summary: &str,
    collapsed: bool,
    section_id: Section,
    pip_state: Option<(bool, Color)>,
    detail: Option<Element<'a, app::Message>>,
    body: Element<'a, app::Message>,
) -> Element<'a, app::Message> {
    use super::interrupts::pip;

    let mut header_left = Vec::new();

    if let Some((active, color)) = pip_state {
        header_left.push(pip(active, color));
    }

    header_left.push(
        text(label)
            .font(fonts::title())
            .size(13.0)
            .color(palette::MUTED)
            .into(),
    );

    // Right side: collapsed summary, or expanded detail if provided
    let header_right: Element<'_, app::Message> = if collapsed {
        text(summary.to_owned())
            .font(fonts::monospace())
            .size(DETAIL)
            .color(palette::OVERLAY0)
            .into()
    } else if let Some(detail) = detail {
        detail
    } else {
        Space::new().into()
    };

    let header = button(
        container(
            row(header_left)
                .push(Space::new().width(Length::Fill))
                .push(header_right)
                .spacing(xs())
                .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .padding([xs(), s()])
        .style(section_header_style),
    )
    .on_press(Message::ToggleSection(section_id).into())
    .padding(0)
    .style(|_, _| button::Style::default())
    .width(Length::Fill);

    let mut content = column![header].width(Length::Fill);
    if !collapsed {
        content = content.push(body);
    }

    container(content)
        .width(Length::Fill)
        .style(section_style)
        .into()
}

fn section_style(theme: &iced::Theme) -> container::Style {
    let pal = theme.extended_palette();
    container::Style {
        background: Some(pal.background.base.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
        ..Default::default()
    }
}

fn section_header_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.03).into()),
        ..Default::default()
    }
}

fn mode_display(mode: missingno_gb::ppu::rendering::Mode) -> (&'static str, Color) {
    use missingno_gb::ppu::rendering::Mode;
    match mode {
        Mode::HorizontalBlank => ("HBlank", palette::BLUE),
        Mode::VerticalBlank => ("VBlank", palette::GREEN),
        Mode::OamScan => ("OAM Scan", palette::YELLOW),
        Mode::Drawing => ("Drawing", palette::PEACH),
    }
}

pub fn tooltip_style(theme: &iced::Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border::default()
            .rounded(4.0)
            .width(1.0)
            .color(palette.background.strong.color),
        ..Default::default()
    }
}

// --- Pointers + halt ---

fn pointers(cpu: &Cpu) -> Element<'_, app::Message> {
    let halted = cpu.halt_state == HaltState::Halted;
    let pc_color = if halted {
        palette::OVERLAY0
    } else {
        palette::PURPLE
    };

    let pc_display: Element<'_, app::Message> = row![
        text("pc")
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(format!("{:04X}", cpu.bus_counter))
            .font(fonts::monospace())
            .size(20.0)
            .color(pc_color),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into();

    let pc_element: Element<'_, app::Message> = if halted {
        tooltip(
            pc_display,
            container(text("halted").font(fonts::monospace()).size(REG)).padding([2.0, s()]),
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into()
    } else {
        pc_display
    };

    row![
        pc_element,
        pointer("sp", format!("{:04X}", cpu.stack_pointer)),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn pointer(label: &str, value: String) -> Element<'_, app::Message> {
    row![
        text(label)
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(value)
            .font(fonts::monospace())
            .size(20.0)
            .color(palette::PURPLE),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

// --- Registers ---

/// Fixed width for one 8-bit register display ("b 04"), so columns align.
const REG8_WIDTH: f32 = 48.0;

fn register_a_row(cpu: &Cpu) -> Element<'_, app::Message> {
    row![
        container(register8(cpu, Register8::A)).width(Length::Fixed(REG8_WIDTH)),
        container("").width(Length::Fixed(REG8_WIDTH)),
        compound_register(cpu, Register16::Af),
        flags_display(cpu.flags),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn flags_display(flags: Flags) -> Element<'static, app::Message> {
    row![
        flag_char("Z", flags.contains(Flags::ZERO)),
        flag_char("N", flags.contains(Flags::NEGATIVE)),
        flag_char("H", flags.contains(Flags::HALF_CARRY)),
        flag_char("C", flags.contains(Flags::CARRY)),
    ]
    .spacing(2.0)
    .into()
}

fn flag_char(label: &str, set: bool) -> Element<'_, app::Message> {
    let (display, color) = if set {
        (label, palette::TEXT)
    } else {
        ("\u{00B7}", palette::SURFACE2) // middle dot
    };
    text(display)
        .font(fonts::monospace())
        .size(REG)
        .color(color)
        .into()
}

fn register_pair_row(
    cpu: &Cpu,
    reg1: Register8,
    reg2: Register8,
    pair: Register16,
) -> Element<'_, app::Message> {
    row![
        container(register8(cpu, reg1)).width(Length::Fixed(REG8_WIDTH)),
        container(register8(cpu, reg2)).width(Length::Fixed(REG8_WIDTH)),
        compound_register(cpu, pair),
    ]
    .spacing(s())
    .align_y(Vertical::Center)
    .into()
}

fn register8(cpu: &Cpu, register: Register8) -> Element<'_, app::Message> {
    row![
        text(register.to_string())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::MUTED),
        text(format!("{:02X}", cpu.get_register8(register)))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::TEXT),
    ]
    .spacing(s())
    .into()
}

fn compound_register(cpu: &Cpu, register: Register16) -> Element<'_, app::Message> {
    row![
        text(register.to_string())
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
        text(format!("{:04X}", cpu.get_register16(register)))
            .font(fonts::monospace())
            .size(REG)
            .color(palette::OVERLAY0),
    ]
    .spacing(s())
    .into()
}
