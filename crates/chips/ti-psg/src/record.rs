//! The PSG's state in the hardware-named state vocabulary: the fields a
//! board's schema carries for the part, and the capture and restore through a
//! [`StateRecord`] keyed on them.

use missingno_core::state::{FieldDef, FieldType, StateRecord, StateValue};
use missingno_core::system::StateError;

use crate::{Channel, NoiseMode, NoiseRate, NoiseState, PsgState, RegisterKind, ToneState};

use FieldType::{Bool, U8, U16};

/// The register file as observable fields, then the counters, flip-flops and
/// shift register a bit-exact restore also needs.
pub fn state_fields() -> Vec<FieldDef> {
    let mut fields = Vec::new();
    for (period, attenuation) in [
        ("psg_tone1_period", "psg_tone1_attenuation"),
        ("psg_tone2_period", "psg_tone2_attenuation"),
        ("psg_tone3_period", "psg_tone3_attenuation"),
    ] {
        fields.push(FieldDef::observable(period, U16, "psg").help("10-bit period register"));
        fields
            .push(FieldDef::observable(attenuation, U8, "psg").help("4-bit attenuation register"));
    }
    fields.push(
        FieldDef::observable("psg_noise_attenuation", U8, "psg").help("4-bit attenuation register"),
    );
    fields.push(
        FieldDef::observable("psg_noise_control", U8, "psg")
            .help("noise register: feedback in bit 2, shift rate in bits 1-0"),
    );
    fields.push(
        FieldDef::observable("psg_latched_register", U8, "psg").help(
            "the register address held between transfers (channel in bits 2-1, type in bit 0)",
        ),
    );

    for (counter, output) in [
        ("psg_tone1_counter", "psg_tone1_output"),
        ("psg_tone2_counter", "psg_tone2_output"),
        ("psg_tone3_counter", "psg_tone3_output"),
    ] {
        fields.push(FieldDef::boundary(counter, U16, "psg").help("the counter toward its borrow"));
        fields.push(FieldDef::boundary(output, Bool, "psg").help("the frequency flip-flop"));
    }
    fields.extend([
        FieldDef::boundary("psg_noise_counter", U16, "psg").help("the counter toward its borrow"),
        FieldDef::boundary("psg_noise_output", Bool, "psg")
            .help("the flip-flop clocking the shift register"),
        FieldDef::boundary("psg_noise_shift_register", U16, "psg")
            .help("the noise shift register's contents"),
        FieldDef::boundary("psg_clock_divider", U8, "psg")
            .help("the ÷16 prescaler's count toward an internal clock"),
        FieldDef::boundary("psg_ready_countdown", U8, "psg")
            .help("input clocks left of the byte load holding READY low"),
    ]);

    fields
}

pub fn write_state(r: &mut StateRecord, psg: &PsgState) {
    for (index, (period, counter, output, attenuation)) in TONE_FIELDS.into_iter().enumerate() {
        r.set(period, psg.tones[index].period)
            .set(counter, psg.tones[index].counter)
            .set(output, psg.tones[index].output)
            .set(attenuation, psg.attenuations[index]);
    }
    r.set("psg_noise_attenuation", psg.attenuations[3])
        .set(
            "psg_noise_control",
            psg.noise.mode.bits() | psg.noise.rate.bits(),
        )
        .set(
            "psg_latched_register",
            latched_register(psg.latched_channel, psg.latched_kind),
        )
        .set("psg_noise_counter", psg.noise.counter)
        .set("psg_noise_output", psg.noise.output)
        .set("psg_noise_shift_register", psg.noise.shift_register)
        .set("psg_clock_divider", psg.clock_divider)
        .set("psg_ready_countdown", psg.ready_countdown);
}

pub fn parse_state(r: &StateRecord) -> Result<PsgState, StateError> {
    let mut tones = [ToneState {
        period: 0,
        counter: 0,
        output: true,
    }; 3];
    let mut attenuations = [0u8; 4];
    for (index, (period, counter, output, attenuation)) in TONE_FIELDS.into_iter().enumerate() {
        tones[index] = ToneState {
            period: u16_of(r, period)?,
            counter: u16_of(r, counter)?,
            output: bool_of(r, output)?,
        };
        attenuations[index] = u8_of(r, attenuation)?;
    }
    attenuations[3] = u8_of(r, "psg_noise_attenuation")?;
    let control = u8_of(r, "psg_noise_control")?;
    let latched = u8_of(r, "psg_latched_register")?;
    Ok(PsgState {
        latched_channel: Channel::ALL[(latched as usize >> 1) & 0x03],
        latched_kind: match latched & 1 {
            0 => RegisterKind::Frequency,
            _ => RegisterKind::Attenuation,
        },
        tones,
        noise: NoiseState {
            rate: NoiseRate::from_control(control),
            mode: NoiseMode::from_control(control),
            counter: u16_of(r, "psg_noise_counter")?,
            output: bool_of(r, "psg_noise_output")?,
            shift_register: u16_of(r, "psg_noise_shift_register")?,
        },
        attenuations,
        clock_divider: u8_of(r, "psg_clock_divider")?,
        ready_countdown: u8_of(r, "psg_ready_countdown")?,
    })
}

/// Per tone generator: period register, counter, flip-flop, attenuation.
const TONE_FIELDS: [(&str, &str, &str, &str); 3] = [
    (
        "psg_tone1_period",
        "psg_tone1_counter",
        "psg_tone1_output",
        "psg_tone1_attenuation",
    ),
    (
        "psg_tone2_period",
        "psg_tone2_counter",
        "psg_tone2_output",
        "psg_tone2_attenuation",
    ),
    (
        "psg_tone3_period",
        "psg_tone3_counter",
        "psg_tone3_output",
        "psg_tone3_attenuation",
    ),
];

fn latched_register(channel: Channel, kind: RegisterKind) -> u8 {
    let kind = match kind {
        RegisterKind::Frequency => 0,
        RegisterKind::Attenuation => 1,
    };
    (channel.index() as u8) << 1 | kind
}

fn u8_of(r: &StateRecord, name: &str) -> Result<u8, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u8::MAX as u32 => Ok(*value as u8),
        _ => Err(StateError::Corrupt),
    }
}

fn u16_of(r: &StateRecord, name: &str) -> Result<u16, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u16::MAX as u32 => Ok(*value as u16),
        _ => Err(StateError::Corrupt),
    }
}

fn bool_of(r: &StateRecord, name: &str) -> Result<bool, StateError> {
    match r.get(name) {
        Some(StateValue::Bool(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}
