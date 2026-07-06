//! Headless console tests: a hand-assembled NTSC kernel drives VSYNC /
//! VBLANK / WSYNC and paints a per-line backdrop gradient.

use missingno_vcs::console::Vcs;

const VSYNC: u8 = 0x00;
const VBLANK: u8 = 0x01;
const WSYNC: u8 = 0x02;
const COLUBK: u8 = 0x09;

/// Just enough of an assembler for test kernels.
struct Asm {
    origin: u16,
    bytes: Vec<u8>,
}

impl Asm {
    fn new(origin: u16) -> Self {
        Asm {
            origin,
            bytes: Vec::new(),
        }
    }

    fn here(&self) -> u16 {
        self.origin + self.bytes.len() as u16
    }

    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn cld(&mut self) {
        self.emit(&[0xD8]);
    }
    fn lda_imm(&mut self, value: u8) {
        self.emit(&[0xA9, value]);
    }
    fn ldx_imm(&mut self, value: u8) {
        self.emit(&[0xA2, value]);
    }
    fn txs(&mut self) {
        self.emit(&[0x9A]);
    }
    fn sta_zp(&mut self, address: u8) {
        self.emit(&[0x85, address]);
    }
    fn stx_zp(&mut self, address: u8) {
        self.emit(&[0x86, address]);
    }
    fn inx(&mut self) {
        self.emit(&[0xE8]);
    }
    fn dex(&mut self) {
        self.emit(&[0xCA]);
    }
    fn cpx_imm(&mut self, value: u8) {
        self.emit(&[0xE0, value]);
    }
    fn bne_to(&mut self, target: u16) {
        let offset = target as i32 - (self.here() as i32 + 2);
        self.emit(&[0xD0, i8::try_from(offset).unwrap() as u8]);
    }
    fn jmp_abs(&mut self, target: u16) {
        self.emit(&[0x4C, target as u8, (target >> 8) as u8]);
    }

    /// Pad to 4 KB with the reset vector pointing at the origin.
    fn into_rom(self) -> Vec<u8> {
        let mut rom = self.bytes;
        rom.resize(0x1000, 0);
        rom[0xFFC] = self.origin as u8;
        rom[0xFFD] = (self.origin >> 8) as u8;
        rom
    }
}

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
    let mut vcs = Vcs::new(&gradient_kernel()).unwrap();

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
    let mut vcs = Vcs::new(&gradient_kernel()).unwrap();
    for _ in 0..5 {
        let frame = vcs.step_frame(1000).expect("steady stream of frames");
        assert_eq!(frame.lines.len(), 259);
    }
}

#[test]
fn wsync_parks_the_cpu_until_line_start() {
    // STA WSYNC mid-line; the following STX must execute from line start.
    let mut asm = Asm::new(0xF000);
    asm.lda_imm(0x02);
    asm.sta_zp(WSYNC);
    asm.stx_zp(COLUBK);
    let spin = asm.here();
    asm.jmp_abs(spin);
    let mut vcs = Vcs::new(&asm.into_rom()).unwrap();

    vcs.step_instruction(); // the reset sequence
    vcs.step_instruction(); // LDA
    vcs.step_instruction(); // STA WSYNC
    vcs.step_instruction(); // STX, parked first
    // Release at line start: STX's cycles land at colour clocks 0/3/6,
    // and the boundary surfaces one TIA clock after the write cycle.
    assert_eq!(vcs.tia.beam(), 7);
}

#[test]
fn budget_guard_returns_none_without_vsync() {
    // A kernel that never syncs: tight spin.
    let mut asm = Asm::new(0xF000);
    let spin = asm.here();
    asm.jmp_abs(spin);
    let mut vcs = Vcs::new(&asm.into_rom()).unwrap();
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

    let mut debugger = Debugger::new(missingno_vcs::console::Vcs::new(&rom).unwrap());
    debugger.set_breakpoint(target);
    let (_, stop) = debugger.run();
    assert_eq!(stop, Stop::Breakpoint);
    assert_eq!(debugger.console().cpu.pc & 0x1FFF, target & 0x1FFF);

    // Repeated peeks at the RIOT timer flag and TIA inputs are inert.
    let first = debugger.console().peek(0x0285);
    let again = debugger.console().peek(0x0285);
    assert_eq!(first, again);
    assert_eq!(debugger.console().peek(0x000C), 0x80, "trigger unpressed");
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
