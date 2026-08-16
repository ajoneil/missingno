//! The SN76489AN's register view, as the part states it. The board supplies
//! the CLOCK pin frequency the tone arithmetic needs.

use missingno_core::inspect::Section;
use missingno_ti_psg::inspect::Registers;

use super::Sg1000InspectState;
use crate::console::CLOCK_HZ;

pub(crate) fn section(state: &Sg1000InspectState) -> Section {
    missingno_ti_psg::inspect::section(
        &Registers {
            tone_periods: state.psg_periods,
            attenuations: state.psg_volumes,
            noise_mode: state.psg_noise_mode,
            noise_rate: state.psg_noise_rate,
            variant: state.psg_variant,
        },
        CLOCK_HZ,
    )
}

#[cfg(test)]
mod tests {
    use missingno_ti_psg::{NoiseMode, NoiseRate};

    use super::*;
    use crate::debug::fixtures::{power_on_state, rows, value_of};

    #[test]
    fn psg_section_pairs_registers_with_their_arithmetic() {
        let state = Sg1000InspectState {
            psg_periods: [0x0FE, 0x1FE, 0],
            // Tone 1 audible (attenuation 0), the rest muted at $F.
            psg_volumes: [0x00, 0x0F, 0x0F, 0x0F],
            psg_noise_mode: NoiseMode::White,
            psg_noise_rate: NoiseRate::Div1024,
            ..power_on_state()
        };
        let section = section(&state);
        assert_eq!(section.name, "PSG");
        assert_eq!(section.summary, "1/4 audible");

        let rows = rows(&section);
        assert_eq!(value_of(&rows, "per1"), Some("0FE (440 Hz)"));
        assert_eq!(value_of(&rows, "att1"), Some("0 (0 dB)"));
        assert_eq!(value_of(&rows, "per2"), Some("1FE (219 Hz)"));
        assert_eq!(value_of(&rows, "att2"), Some("F (off)"));
        // The discrete part's zero period is a full-span count, not silence.
        assert_eq!(value_of(&rows, "per3"), Some("000 (109 Hz)"));
        assert_eq!(value_of(&rows, "mode"), Some("white"));
        assert_eq!(value_of(&rows, "rate"), Some("clock ÷ 1024"));
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 1")
                .and_then(|row| row.active),
            Some(true)
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 2")
                .and_then(|row| row.active),
            Some(false)
        );
    }
}
