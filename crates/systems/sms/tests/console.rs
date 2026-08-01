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

/// Set the VDP write address (code 1 for VRAM, then data-port bytes).
fn set_vram_address(asm: &mut Asm, address: u16) {
    asm.ld_a(address as u8);
    asm.out_port(0xBF);
    asm.ld_a(0x40 | (address >> 8) as u8);
    asm.out_port(0xBF);
}

fn write_vram(asm: &mut Asm, address: u16, bytes: &[u8]) {
    set_vram_address(asm, address);
    for &byte in bytes {
        asm.ld_a(byte);
        asm.out_port(0xBE);
    }
}

fn write_cram(asm: &mut Asm, index: u8, value: u8) {
    asm.ld_a(index);
    asm.out_port(0xBF);
    asm.ld_a(0xC0);
    asm.out_port(0xBF);
    asm.ld_a(value);
    asm.out_port(0xBE);
}

/// Standard test scene: nametable at $3800, SAT at $3F00, display on.
fn mode4_setup(asm: &mut Asm) {
    set_vdp_register(asm, 2, 0x0E);
    set_vdp_register(asm, 5, 0x7E);
    set_vdp_register(asm, 1, 0x40);
}

/// A solid tile: every pixel is `color` (one byte per plane per row).
fn solid_tile_bytes(color: u8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    for _ in 0..8 {
        for plane in 0..4 {
            bytes.push(if color & (1 << plane) != 0 {
                0xFF
            } else {
                0x00
            });
        }
    }
    bytes
}

fn frame_after_setup(rom: Vec<u8>) -> missingno_sms::vdp::Frame {
    let mut sms = Sms::new(&rom).unwrap();
    let _ = sms.step_frame(FRAME_BUDGET).expect("first frame");
    sms.step_frame(FRAME_BUDGET).expect("second frame")
}

#[test]
fn background_tile_renders_at_the_origin() {
    let mut asm = Asm::new();
    mode4_setup(&mut asm);
    write_vram(&mut asm, 0x20, &solid_tile_bytes(5)); // tile 1
    write_vram(&mut asm, 0x3800, &[0x01, 0x00]); // nametable (0,0) = tile 1
    write_cram(&mut asm, 5, 0x3F);
    let spin = asm.here();
    asm.jp(spin);

    let frame = frame_after_setup(asm.into_rom());
    assert!(frame.pixels[..8].iter().all(|&p| p == 5), "tile colour 5");
    assert_eq!(frame.pixels[8], 0, "rest of the line is tile 0, colour 0");
    let line7 = &frame.pixels[7 * 256..7 * 256 + 8];
    assert!(line7.iter().all(|&p| p == 5), "all 8 tile rows");
    assert_eq!(frame.pixels[8 * 256], 0, "next tile row is empty");
}

#[test]
fn sprite_renders_in_the_second_palette() {
    let mut asm = Asm::new();
    mode4_setup(&mut asm);
    write_vram(&mut asm, 2 * 32, &solid_tile_bytes(3)); // tile 2
    write_vram(&mut asm, 0x3F00, &[9, 0xD0]); // sprite 0 y=9 (top=10), terminator
    write_vram(&mut asm, 0x3F80, &[100, 2]); // sprite 0: x=100, tile 2
    let spin = asm.here();
    asm.jp(spin);

    let frame = frame_after_setup(asm.into_rom());
    let line12 = &frame.pixels[12 * 256..13 * 256];
    assert!(
        line12[100..108].iter().all(|&p| p == 16 + 3),
        "sprite colour 3 in the sprite palette, got {:?}",
        &line12[100..108]
    );
    assert_eq!(line12[99], 0);
    assert_eq!(line12[108], 0);
    let line9 = &frame.pixels[9 * 256..10 * 256];
    assert!(line9.iter().all(|&p| p == 0), "sprite starts at y+1");
    let line18 = &frame.pixels[18 * 256..19 * 256];
    assert!(
        line18.iter().all(|&p| p == 0),
        "8x8 sprite ends after 8 lines"
    );
}

#[test]
fn horizontal_scroll_shifts_the_background() {
    let mut asm = Asm::new();
    mode4_setup(&mut asm);
    write_vram(&mut asm, 0x20, &solid_tile_bytes(5));
    write_vram(&mut asm, 0x3800, &[0x01, 0x00]);
    set_vdp_register(&mut asm, 8, 4); // scroll right 4
    let spin = asm.here();
    asm.jp(spin);

    let frame = frame_after_setup(asm.into_rom());
    assert_eq!(frame.pixels[3], 0, "vacated by the shift");
    assert!(
        frame.pixels[4..12].iter().all(|&p| p == 5),
        "tile moved right by 4, got {:?}",
        &frame.pixels[..14]
    );
    assert_eq!(frame.pixels[12], 0);
}

#[test]
fn tile_priority_covers_sprites_except_through_colour_zero() {
    let mut asm = Asm::new();
    mode4_setup(&mut asm);
    // Tile 1: left half colour 5, right half colour 0 (per-row bytes:
    // plane0/2 form 0xF0 where colour-5 bits sit).
    let mut tile = Vec::new();
    for _ in 0..8 {
        tile.extend_from_slice(&[0xF0, 0x00, 0xF0, 0x00]); // colour 5 = planes 0+2
    }
    write_vram(&mut asm, 0x20, &tile);
    // Nametable (0,0): tile 1 with the priority bit.
    write_vram(&mut asm, 0x3800, &[0x01, 0x10]);
    write_vram(&mut asm, 2 * 32, &solid_tile_bytes(3));
    write_vram(&mut asm, 0x3F00, &[0xFF, 0xD0]); // y=$FF: top = 0
    write_vram(&mut asm, 0x3F80, &[0, 2]); // sprite at x=0
    let spin = asm.here();
    asm.jp(spin);

    let frame = frame_after_setup(asm.into_rom());
    let line0 = &frame.pixels[..16];
    assert!(
        line0[..4].iter().all(|&p| p == 5),
        "priority tile covers the sprite, got {line0:?}"
    );
    assert!(
        line0[4..8].iter().all(|&p| p == 16 + 3),
        "sprite shows through the tile's colour 0, got {line0:?}"
    );
}
