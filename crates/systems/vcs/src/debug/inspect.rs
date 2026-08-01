//! The inspection state both seam halves render from: one owned struct,
//! captured from the console peek-only, serving the paused view (refreshed
//! after every step) and the per-frame snapshot the running view reads.

use rgb::RGB8;

use crate::console::Vcs;
use crate::tia::palette_index;

#[derive(Clone, Default)]
pub struct VcsInspectState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    pub beam: u16,
    pub scanline: usize,
    pub timer: u8,
    pub timer_underflowed: bool,
    pub swcha: u8,
    pub swchb: u8,
    pub collisions: [u8; 8],
    /// TIA graphics registers, resolved to their object colours for the pixel
    /// strips (COLUPx is a hue the core owns).
    pub grp0: u8,
    pub grp0_reflect: bool,
    pub grp1: u8,
    pub grp1_reflect: bool,
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub pf_mirrored: bool,
    pub missile0: bool,
    pub missile1: bool,
    pub ball: bool,
    pub color_p0: RGB8,
    pub color_p1: RGB8,
    pub color_pf: RGB8,
    /// TIA audio: each channel's AUDC/AUDF/AUDV register bytes.
    pub audc: [u8; 2],
    pub audf: [u8; 2],
    pub audv: [u8; 2],
    /// The board and, on a DPC cart, its custom chip.
    pub cartridge: crate::cartridge::CartridgeInspect,
    pub frame: u64,
}

/// Read the inspection state out of a console without disturbing it.
pub(super) fn capture(vcs: &Vcs, frame: u64) -> VcsInspectState {
    let cpu = &vcs.cpu;
    let standard = vcs.tv_standard();
    let color = |byte: u8| {
        let (r, g, b) = crate::tia::palette(standard)[palette_index(byte)];
        RGB8::new(r, g, b)
    };
    let gfx = vcs.tia.graphics_registers();
    let audio = vcs.tia.audio_registers();
    VcsInspectState {
        a: cpu.a,
        x: cpu.x,
        y: cpu.y,
        s: cpu.s,
        p: cpu.p,
        pc: cpu.pc,
        beam: vcs.tia.beam(),
        scanline: vcs.scanline(),
        timer: vcs.peek(0x0284),
        timer_underflowed: vcs.peek(0x0285) & 0x80 != 0,
        swcha: vcs.peek(0x0280),
        swchb: vcs.peek(0x0282),
        collisions: std::array::from_fn(|i| vcs.peek(i as u16)),
        grp0: gfx.grp0,
        grp0_reflect: gfx.reflect_p0,
        grp1: gfx.grp1,
        grp1_reflect: gfx.reflect_p1,
        pf0: gfx.pf0,
        pf1: gfx.pf1,
        pf2: gfx.pf2,
        pf_mirrored: gfx.pf_mirrored,
        missile0: gfx.missile0,
        missile1: gfx.missile1,
        ball: gfx.ball,
        color_p0: color(gfx.color_p0),
        color_p1: color(gfx.color_p1),
        color_pf: color(gfx.color_pf),
        audc: [audio[0].control, audio[1].control],
        audf: [audio[0].frequency, audio[1].frequency],
        audv: [audio[0].volume, audio[1].volume],
        cartridge: vcs.cartridge().inspect(),
        frame,
    }
}
