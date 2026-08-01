use missingno_gb::{WaveRamCoupling, audio::ApuSpec};

/// CGB APU spec: KEY1 double-speed, the widened CH1 sweep load-hold, the CH4
/// divisor-code grid anchor, and channel-position wave-RAM coupling.
#[derive(Clone, Copy, Default)]
pub struct CgbApu;
impl ApuSpec for CgbApu {
    const DOUBLE_SPEED: bool = true;
    const WIDE_SWEEP_LOAD_HOLD: bool = true;
    const NOISE_GRID_ANCHOR: bool = true;
    const WAVE_RAM_COUPLING: WaveRamCoupling = WaveRamCoupling::ChannelPosition;
}
