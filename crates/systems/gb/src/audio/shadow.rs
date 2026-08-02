//! Shadow verification of the skipped span.
//!
//! A slow-path copy taken at span entry is ticked through every edge the fast
//! path skips; at materialisation the reconstruction has to land on exactly the
//! state that copy reached, field for field. Debug builds only — the whole
//! module is the machine-checked half of the equivalence claim.

use std::marker::PhantomData;

use missingno_core::waveform::WaveRing;

use super::span::SpanPredictor;
use super::{ApuSpec, Audio};

impl<A: ApuSpec> Audio<A> {
    /// Drive the shadow through one skipped edge, taking the copy on the first.
    pub(super) fn shadow_tcycle(&mut self, div_counter: u16, t_index: u8, double_speed: bool) {
        if self.shadow.is_none() {
            self.shadow = Some(Box::new(self.slow_path_copy()));
        }
        if let Some(shadow) = &mut self.shadow {
            shadow.tcycle(div_counter, t_index, double_speed);
        }
    }

    /// A copy of this APU that never skips. Spelled out rather than derived:
    /// `Clone` on `Audio<A>` carries an `A: Clone` bound this context lacks.
    fn slow_path_copy(&self) -> Self {
        Self {
            enabled: self.enabled,
            channel_clock: self.channel_clock.clone(),
            channels: self.channels.clone(),
            volume_left: self.volume_left,
            volume_right: self.volume_right,
            nr50: self.nr50,
            prev_div_apu_bit: self.prev_div_apu_bit,
            frame_sequencer_step: self.frame_sequencer_step,
            fs_edge_pending: self.fs_edge_pending,
            div_apu_double_parity: self.div_apu_double_parity,
            div_apu_switch_lag: self.div_apu_switch_lag,
            fs_edge_predelay: self.fs_edge_predelay,
            sample_counter: self.sample_counter,
            pending_left: self.pending_left,
            pending_right: self.pending_right,
            pending_count: self.pending_count,
            last_mix: self.last_mix,
            mix_run: self.mix_run,
            sample_accum_left: self.sample_accum_left,
            sample_accum_right: self.sample_accum_right,
            sample_accum_count: self.sample_accum_count,
            // Only the span's own pushes are compared, so the copy starts empty.
            sample_buffer: Vec::new(),
            wave_capture: self.wave_capture.clone(),
            span: SpanPredictor::shadow(),
            shadow: None,
            _spec: PhantomData,
        }
    }

    pub(super) fn shadow_fall_sync(&mut self) {
        if let Some(shadow) = &mut self.shadow {
            shadow.fall_sync();
        }
    }

    pub(super) fn shadow_mcycle_boundary(&mut self) {
        if let Some(shadow) = &mut self.shadow {
            shadow.mcycle_boundary();
        }
    }

    /// Compare the reconstruction against the shadow and drop it. Destructuring
    /// keeps the field list exhaustive: a new field breaks this build.
    pub(super) fn check_shadow(&mut self) {
        let Some(shadow) = self.shadow.take() else {
            return;
        };
        let Self {
            enabled,
            channel_clock,
            channels,
            volume_left,
            volume_right,
            nr50,
            prev_div_apu_bit,
            frame_sequencer_step,
            fs_edge_pending,
            div_apu_double_parity,
            div_apu_switch_lag,
            fs_edge_predelay,
            sample_counter,
            pending_left,
            pending_right,
            pending_count,
            last_mix,
            mix_run,
            sample_accum_left,
            sample_accum_right,
            sample_accum_count,
            sample_buffer,
            wave_capture,
            span: _,
            shadow: _,
            _spec: _,
        } = self;

        macro_rules! same {
            ($field:ident) => {
                assert!(
                    *$field == shadow.$field,
                    concat!("materialised ", stringify!($field), ": {:?} vs shadow {:?}"),
                    $field,
                    shadow.$field
                );
            };
        }

        same!(enabled);
        same!(channel_clock);
        same!(volume_left);
        same!(volume_right);
        same!(nr50);
        same!(prev_div_apu_bit);
        same!(frame_sequencer_step);
        same!(fs_edge_pending);
        same!(div_apu_double_parity);
        same!(div_apu_switch_lag);
        same!(fs_edge_predelay);
        same!(sample_counter);
        same!(pending_left);
        same!(pending_right);
        same!(pending_count);
        same!(last_mix);
        same!(mix_run);
        same!(sample_accum_left);
        same!(sample_accum_right);
        same!(sample_accum_count);
        let pushed = sample_buffer
            .len()
            .saturating_sub(shadow.sample_buffer.len());
        assert!(
            sample_buffer[pushed..] == shadow.sample_buffer[..],
            "materialised sample_buffer: {:?} vs shadow {:?}",
            &sample_buffer[pushed..],
            shadow.sample_buffer
        );
        assert!(
            rings(wave_capture) == rings(&shadow.wave_capture),
            "materialised wave_capture diverged"
        );

        let ch = &shadow.channels;
        macro_rules! same_channel {
            ($channel:ident) => {
                assert!(
                    channels.$channel == ch.$channel,
                    concat!(
                        "materialised ",
                        stringify!($channel),
                        ": {:?}\nvs shadow {:?}"
                    ),
                    channels.$channel,
                    ch.$channel
                );
            };
        }
        same_channel!(ch1);
        same_channel!(ch2);
        same_channel!(ch3);
        same_channel!(ch4);
    }
}

fn rings(capture: &Option<[WaveRing; 4]>) -> Option<Vec<Vec<u8>>> {
    capture
        .as_ref()
        .map(|rings| rings.iter().map(|ring| ring.to_vec()).collect())
}
