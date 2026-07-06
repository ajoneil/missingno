//! Headless console tests: hand-assembled Z80 kernels drive the VDP
//! ports, the mapper, and the interrupt path.

use missingno_sms::console::Sms;
use missingno_sms::vdp;

struct Asm {
    bytes: Vec<u8>,
}

impl Asm {
    fn new() -> Self {
        Asm { bytes: Vec::new() }
    }
    fn here(&self) -> u16 {
        self.bytes.len() as u16
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    fn pad_to(&mut self, address: u16) {
        assert!(self.here() <= address);
        self.bytes.resize(address as usize, 0);
    }
    fn ld_a(&mut self, value: u8) {
        self.emit(&[0x3E, value]);
    }
    fn out_port(&mut self, port: u8) {
        self.emit(&[0xD3, port]);
    }
    fn in_port(&mut self, port: u8) {
        self.emit(&[0xDB, port]);
    }
    fn ld_addr_a(&mut self, address: u16) {
        self.emit(&[0x32, address as u8, (address >> 8) as u8]);
    }
    fn ld_a_addr(&mut self, address: u16) {
        self.emit(&[0x3A, address as u8, (address >> 8) as u8]);
    }
    fn inc_a(&mut self) {
        self.emit(&[0x3C]);
    }
    fn ei(&mut self) {
        self.emit(&[0xFB]);
    }
    fn im1(&mut self) {
        self.emit(&[0xED, 0x56]);
    }
    fn ret(&mut self) {
        self.emit(&[0xC9]);
    }
    fn jp(&mut self, target: u16) {
        self.emit(&[0xC3, target as u8, (target >> 8) as u8]);
    }
    /// Pad to one 16 KB page.
    fn into_rom(mut self) -> Vec<u8> {
        self.bytes.resize(0x4000, 0);
        self.bytes
    }
}

/// Write a VDP register through the control port.
fn set_vdp_register(asm: &mut Asm, register: u8, value: u8) {
    asm.ld_a(value);
    asm.out_port(0xBF);
    asm.ld_a(0x80 | register);
    asm.out_port(0xBF);
}

const FRAME_BUDGET: u32 = 200_000;

#[test]
fn backdrop_frame_carries_cram_snapshot() {
    let mut asm = Asm::new();
    // Backdrop = sprite-palette entry 2; CRAM[18] = bright green (%001100).
    set_vdp_register(&mut asm, 7, 0x02);
    // CRAM write: address 18, code 3.
    asm.ld_a(18);
    asm.out_port(0xBF);
    asm.ld_a(0xC0);
    asm.out_port(0xBF);
    asm.ld_a(0x0C);
    asm.out_port(0xBE);
    let spin = asm.here();
    asm.jp(spin);

    let mut sms = Sms::new(&asm.into_rom()).unwrap();
    let _ = sms.step_frame(FRAME_BUDGET).expect("first frame");
    let frame = sms.step_frame(FRAME_BUDGET).expect("second frame");
    assert_eq!(
        frame.pixels.len(),
        vdp::PIXELS_PER_LINE * vdp::ACTIVE_LINES as usize
    );
    assert!(frame.pixels.iter().all(|&p| p == 18), "backdrop index 18");
    assert_eq!(frame.cram[18], 0x0C, "CRAM snapshot rides the frame");
}

#[test]
fn frame_interrupt_reaches_an_im1_handler() {
    let mut asm = Asm::new();
    asm.jp(0x0100);

    // The IM 1 vector: acknowledge the VDP and count interrupts in RAM.
    asm.pad_to(0x0038);
    asm.in_port(0xBF);
    asm.ld_a_addr(0xC000);
    asm.inc_a();
    asm.ld_addr_a(0xC000);
    asm.ei();
    asm.ret();

    asm.pad_to(0x0100);
    asm.ld_a(0x00);
    asm.ld_addr_a(0xC000);
    set_vdp_register(&mut asm, 1, 0x20); // frame interrupts on
    asm.im1();
    asm.ei();
    let spin = asm.here();
    asm.jp(spin);

    let mut sms = Sms::new(&asm.into_rom()).unwrap();
    for _ in 0..3 {
        let _ = sms.step_frame(FRAME_BUDGET).expect("frame");
    }
    let count = sms.peek(0xC000);
    assert!(
        (2..=3).contains(&count),
        "one interrupt per completed frame, got {count}"
    );
}

#[test]
fn sega_mapper_banks_slot_one() {
    let mut asm = Asm::new();
    // Write bank 2 into slot 1 ($FFFE), then copy $4000 into RAM.
    asm.ld_a(0x02);
    asm.ld_addr_a(0xFFFE);
    asm.ld_a_addr(0x4000);
    asm.ld_addr_a(0xC010);
    let spin = asm.here();
    asm.jp(spin);

    let mut rom = asm.into_rom();
    rom.resize(0x4000 * 3, 0);
    rom[0x4000] = 0x11; // bank 1 marker
    rom[0x8000] = 0x22; // bank 2 marker

    let mut sms = Sms::new(&rom).unwrap();
    for _ in 0..100 {
        sms.step_instruction();
    }
    assert_eq!(sms.peek(0xC010), 0x22, "slot 1 sees bank 2 after the latch");
    // The latch write also landed in the RAM mirror.
    assert_eq!(sms.peek(0xFFFE), 0x02);
    assert_eq!(sms.peek(0xDFFE), 0x02, "mirror shares the cell");
}

#[test]
fn v_counter_wraps_the_ntsc_gap() {
    let mut sms = Sms::new(&Asm::new().into_rom()).unwrap();
    assert_eq!(sms.vdp.v_counter(), 0);
    for _ in 0..vdp::DOTS_PER_LINE as usize * 219 {
        sms.vdp.step_dot();
    }
    assert_eq!(sms.vdp.line(), 219);
    assert_eq!(sms.vdp.v_counter(), 0xD5, "counts $00-$DA then $D5-$FF");
}

#[test]
fn psg_tone_reaches_the_seam_rate() {
    let mut asm = Asm::new();
    // Channel 0: period latch+data for a mid pitch, full volume.
    asm.ld_a(0x8E); // latch ch0 tone, low bits $E
    asm.out_port(0x7F);
    asm.ld_a(0x0C); // data: upper bits
    asm.out_port(0x7F);
    asm.ld_a(0x90); // ch0 volume: full
    asm.out_port(0x7F);
    let spin = asm.here();
    asm.jp(spin);

    let mut sms = Sms::new(&asm.into_rom()).unwrap();
    // Frames complete at the vblank boundary, so the first window since
    // power-on is partial; measure a full frame-to-frame period.
    let _ = sms.step_frame(FRAME_BUDGET).expect("first frame");
    sms.drain_audio_samples();
    let _ = sms.step_frame(FRAME_BUDGET).expect("second frame");
    let samples = sms.drain_audio_samples();
    assert!(
        (650..820).contains(&samples.len()),
        "one NTSC frame is ~735 samples, got {}",
        samples.len()
    );
    assert!(samples.iter().any(|&(l, _)| l > 0.2), "tone swings high");
    assert!(samples.iter().any(|&(l, _)| l < 0.05), "tone swings low");
}
