//! The register copies the debugger reads. The TIA's picture and audio
//! registers are write-only on the bus, so their values are read out here.

use super::Tia;
use super::objects::Player;

/// The TIA graphics registers driving the picture, copied for the debugger's
/// pixel strips. Write-only on the bus, so the debugger reads them here.
pub struct GraphicsRegisters {
    /// Effective player patterns (the VDELP-selected GRP copy) and their REFP.
    pub grp0: u8,
    pub reflect_p0: bool,
    pub grp1: u8,
    pub reflect_p1: bool,
    /// The playfield's three pattern registers and its CTRLPF reflect bit.
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub pf_mirrored: bool,
    /// Whether each missile / the ball currently draws.
    pub missile0: bool,
    pub missile1: bool,
    pub ball: bool,
    /// The object colour bytes (COLUP0/COLUP1/COLUPF), TIA-palette indices.
    pub color_p0: u8,
    pub color_p1: u8,
    pub color_pf: u8,
}

/// One audio channel's AUDC/AUDF/AUDV register bytes, copied for the debugger.
/// Write-only on the bus, so the debugger reads them here.
pub struct AudioRegisters {
    /// AUDC waveform/tone class (low 4 bits).
    pub control: u8,
    /// AUDF frequency divider (5 bits).
    pub frequency: u8,
    /// AUDV volume (4 bits).
    pub volume: u8,
}

impl Tia {
    /// The graphics registers driving the picture, for the debugger's pixel
    /// strips: the two player patterns (effective GRP after VDELP, plus REFP),
    /// the playfield's three pattern registers and its reflect bit, the
    /// missile/ball enables, and each object's colour byte. Inspection only.
    pub fn graphics_registers(&self) -> GraphicsRegisters {
        let player = |p: &Player| {
            (
                if p.vertical_delay {
                    p.graphics_old
                } else {
                    p.graphics_new
                },
                p.reflect,
            )
        };
        let (grp0, reflect_p0) = player(&self.movables.p0);
        let (grp1, reflect_p1) = player(&self.movables.p1);
        GraphicsRegisters {
            grp0,
            reflect_p0,
            grp1,
            reflect_p1,
            pf0: self.playfield.pf0,
            pf1: self.playfield.pf1,
            pf2: self.playfield.pf2,
            pf_mirrored: self.playfield.mirrored,
            missile0: self.movables.m0.enabled && !self.movables.m0.locked_to_player,
            missile1: self.movables.m1.enabled && !self.movables.m1.locked_to_player,
            ball: self.movables.bl.enabled(),
            color_p0: self.mux.color_p0,
            color_p1: self.mux.color_p1,
            color_pf: self.mux.color_pf,
        }
    }

    /// The two audio channels' AUDC/AUDF/AUDV register bytes. Write-only on the
    /// bus, so the debugger reads them here. Inspection only.
    pub fn audio_registers(&self) -> [AudioRegisters; 2] {
        std::array::from_fn(|i| AudioRegisters {
            control: self.audio[i].control,
            frequency: self.audio[i].frequency,
            volume: self.audio[i].volume,
        })
    }
}
