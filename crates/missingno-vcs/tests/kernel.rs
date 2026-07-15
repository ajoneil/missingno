//! Headless console tests: a hand-assembled NTSC kernel drives VSYNC /
//! VBLANK / WSYNC and paints a per-line backdrop gradient.

use missingno_vcs::console::Vcs;

#[path = "support/asm.rs"]
mod asm;
use asm::Asm;

const VSYNC: u8 = 0x00;
const VBLANK: u8 = 0x01;
const WSYNC: u8 = 0x02;
const COLUBK: u8 = 0x09;

/// 3 VSYNC + 37 VBLANK + 192 gradient + 30 overscan, forever.
fn gradient_kernel() -> Vec<u8> {
    let mut asm = Asm::new(0xF000);
    asm.cld();
    asm.ldx_imm(0xFF);
    asm.txs();

    let frame = asm.here();
    asm.lda_imm(0x02);
    asm.sta_zp(VSYNC);
    for _ in 0..3 {
        asm.sta_zp(WSYNC);
    }
    asm.lda_imm(0x00);
    asm.sta_zp(VSYNC);

    asm.lda_imm(0x02);
    asm.sta_zp(VBLANK);
    asm.ldx_imm(37);
    let vblank_loop = asm.here();
    asm.sta_zp(WSYNC);
    asm.dex();
    asm.bne_to(vblank_loop);
    asm.lda_imm(0x00);
    asm.sta_zp(VBLANK);

    asm.ldx_imm(0);
    let visible_loop = asm.here();
    asm.stx_zp(COLUBK);
    asm.sta_zp(WSYNC);
    asm.inx();
    asm.cpx_imm(192);
    asm.bne_to(visible_loop);

    asm.lda_imm(0x02);
    asm.sta_zp(VBLANK);
    asm.ldx_imm(30);
    let overscan_loop = asm.here();
    asm.sta_zp(WSYNC);
    asm.dex();
    asm.bne_to(overscan_loop);
    asm.jmp_abs(frame);

    asm.into_rom()
}

#[test]
fn gradient_kernel_produces_ntsc_frames() {
    let mut vcs = Vcs::new(&gradient_kernel(), missingno_vcs::TvStandard::Ntsc, None).unwrap();

    // The first frame is the ragged power-on one; judge the second.
    let _ = vcs.step_frame(1000).expect("first frame");
    let frame = vcs.step_frame(1000).expect("second frame");

    assert_eq!(frame.lines.len(), 259, "37 + 192 + 30 non-VSYNC lines");
    for (i, line) in frame.lines[..37].iter().enumerate() {
        assert!(
            line.iter().all(|&p| p == 0),
            "vblank line {i} should be black"
        );
    }
    for i in 0..192 {
        let expected = (i as u8) & 0xFE;
        assert!(
            frame.lines[37 + i].iter().all(|&p| p == expected),
            "gradient line {i}: expected {expected:02X}, got {:02X?}",
            &frame.lines[37 + i][..4]
        );
    }
    for (i, line) in frame.lines[229..].iter().enumerate() {
        assert!(
            line.iter().all(|&p| p == 0),
            "overscan line {i} should be black"
        );
    }
}

#[test]
fn frames_keep_coming() {
    let mut vcs = Vcs::new(&gradient_kernel(), missingno_vcs::TvStandard::Ntsc, None).unwrap();
    for _ in 0..5 {
        let frame = vcs.step_frame(1000).expect("steady stream of frames");
        assert_eq!(frame.lines.len(), 259);
    }
}

#[test]
fn budget_guard_returns_none_without_vsync() {
    // A kernel that never syncs: tight spin.
    let mut asm = Asm::new(0xF000);
    let spin = asm.here();
    asm.jmp_abs(spin);
    let mut vcs = Vcs::new(&asm.into_rom(), missingno_vcs::TvStandard::Ntsc, None).unwrap();
    assert!(vcs.step_frame(400).is_none());
}

#[test]
fn debugger_breakpoints_and_peek_are_side_effect_free() {
    use missingno_vcs::debugger::{Debugger, Stop};

    let mut asm = Asm::new(0xF000);
    asm.lda_imm(0x02);
    asm.sta_zp(0x02); // WSYNC
    let target = asm.here();
    asm.lda_imm(0x2A);
    let spin = asm.here();
    asm.jmp_abs(spin);
    let rom = asm.into_rom();

    let mut debugger = Debugger::new(
        missingno_vcs::console::Vcs::new(&rom, missingno_vcs::TvStandard::Ntsc, None).unwrap(),
    );
    debugger.set_breakpoint(target);
    let (_, stop) = debugger.run();
    assert_eq!(stop, Stop::Breakpoint);
    assert_eq!(debugger.console().cpu.pc & 0x1FFF, target & 0x1FFF);

    // Repeated peeks at the RIOT timer flag and TIA inputs are inert.
    let first = debugger.console().peek(0x0285);
    let again = debugger.console().peek(0x0285);
    assert_eq!(first, again);
    // The TIA drives only D7 on input reads; the low bits float.
    assert_eq!(
        debugger.console().peek(0x000C) & 0x80,
        0x80,
        "trigger unpressed"
    );
}

#[test]
fn disassembles_the_basics() {
    use missingno_vcs::cpu::disasm::disassemble;
    let d = disassemble(0xF000, [0xA9, 0x42, 0x00]);
    assert_eq!(d.mnemonic, "lda #$42");
    assert_eq!(d.length, 2);
    let d = disassemble(0xF000, [0x10, 0xFE, 0x00]);
    assert_eq!(d.mnemonic, "bpl $f000");
    let d = disassemble(0xF000, [0x4C, 0x34, 0x12]);
    assert_eq!(d.mnemonic, "jmp $1234");
    assert_eq!(d.length, 3);
}

#[test]
fn silent_waveform_holds_audv_as_a_steady_level() {
    // AUDC=$00 parks the DAC conducting, so AUDV alone sets the level — the
    // path sample-playback software drives. A held level must survive the
    // window average unchanged.
    let mut asm = Asm::new(0xF000);
    asm.lda_imm(0x00);
    asm.sta_zp(0x15); // AUDC0: silence decode
    asm.lda_imm(0x0F);
    asm.sta_zp(0x19); // AUDV0: full volume
    let spin = asm.here();
    asm.jmp_abs(spin);

    let mut vcs = Vcs::new(&asm.into_rom(), missingno_vcs::TvStandard::Ntsc, None).unwrap();
    for _ in 0..262 * 228 {
        vcs.step_clock();
    }
    let samples = vcs.drain_audio_samples();
    // One channel wide open sits two thirds of the way to full scale.
    let expected = 2.0 / 3.0;
    let settled = &samples[16..];
    assert!(
        settled.iter().all(|&(l, _)| (l - expected).abs() < 1e-5),
        "expected a steady {expected}, got {:?}",
        &settled[..8]
    );
}

#[test]
fn audio_produces_samples_at_the_seam_rate() {
    // Pure tone on channel 0 at moderate pitch, full volume.
    let mut asm = Asm::new(0xF000);
    asm.lda_imm(0x04);
    asm.sta_zp(0x15); // AUDC0: pure tone
    asm.lda_imm(0x10);
    asm.sta_zp(0x17); // AUDF0
    asm.lda_imm(0x0F);
    asm.sta_zp(0x19); // AUDV0: full volume
    let spin = asm.here();
    asm.jmp_abs(spin);

    let mut vcs = Vcs::new(&asm.into_rom(), missingno_vcs::TvStandard::Ntsc, None).unwrap();
    // One NTSC frame's worth of clocks ≈ 262 × 228; expect ~735 samples.
    for _ in 0..262 * 228 {
        vcs.step_clock();
    }
    let samples = vcs.drain_audio_samples();
    assert!(
        (700..800).contains(&samples.len()),
        "expected ~735 samples per frame, got {}",
        samples.len()
    );
    assert!(
        samples.iter().any(|&(l, _)| l > 0.4),
        "tone should reach full-volume level"
    );
    assert!(
        samples.iter().any(|&(l, _)| l < 0.1),
        "square wave should also swing low"
    );
}
