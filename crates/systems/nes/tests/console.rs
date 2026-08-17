//! Headless console tests: hand-assembled kernels drive the PPU ports,
//! NMI, OAM DMA, and the controller shift path.

use missingno_nes::cartridge::CartridgeError;
use missingno_nes::console::Nes;
use missingno_nes::ppu;

struct Asm {
    bytes: Vec<u8>,
}

impl Asm {
    fn new() -> Self {
        Asm { bytes: Vec::new() }
    }
    fn here(&self) -> u16 {
        0x8000 + self.bytes.len() as u16
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    fn pad_to(&mut self, address: u16) {
        assert!(self.here() <= address);
        self.bytes.resize((address - 0x8000) as usize, 0);
    }
    fn lda_imm(&mut self, v: u8) {
        self.emit(&[0xA9, v]);
    }
    fn ldx_imm(&mut self, v: u8) {
        self.emit(&[0xA2, v]);
    }
    fn sta_abs(&mut self, a: u16) {
        self.emit(&[0x8D, a as u8, (a >> 8) as u8]);
    }
    fn lda_abs(&mut self, a: u16) {
        self.emit(&[0xAD, a as u8, (a >> 8) as u8]);
    }
    fn sta_zp(&mut self, a: u8) {
        self.emit(&[0x85, a]);
    }
    fn inc_zp(&mut self, a: u8) {
        self.emit(&[0xE6, a]);
    }
    fn inx(&mut self) {
        self.emit(&[0xE8]);
    }
    fn cpx_imm(&mut self, v: u8) {
        self.emit(&[0xE0, v]);
    }
    fn lsr_a(&mut self) {
        self.emit(&[0x4A]);
    }
    fn rol_zp(&mut self, a: u8) {
        self.emit(&[0x26, a]);
    }
    fn bne_to(&mut self, target: u16) {
        let offset = target as i32 - (self.here() as i32 + 2);
        self.emit(&[0xD0, i8::try_from(offset).unwrap() as u8]);
    }
    fn jmp(&mut self, target: u16) {
        self.emit(&[0x4C, target as u8, (target >> 8) as u8]);
    }
    fn rti(&mut self) {
        self.emit(&[0x40]);
    }

    /// One 16 KB PRG page (mirrored to $C000) plus an 8 KB CHR page.
    fn into_rom(mut self, chr: &[u8]) -> Vec<u8> {
        self.bytes.resize(0x4000, 0);
        let mut rom = Vec::new();
        rom.extend_from_slice(b"NES\x1A");
        rom.extend_from_slice(&[1, 1, 0, 0]);
        rom.extend_from_slice(&[0; 8]);
        rom.extend_from_slice(&self.bytes);
        let mut chr_page = chr.to_vec();
        chr_page.resize(0x2000, 0);
        rom.extend_from_slice(&chr_page);
        rom
    }
}

/// Reset at $8000, NMI vector to `nmi`.
fn finish_rom(mut asm: Asm, nmi: u16, chr: &[u8]) -> Vec<u8> {
    asm.pad_to(0xBFF0);
    let mut rom = asm.into_rom(chr);
    let vectors = 16 + 0x3FFA;
    rom[vectors] = nmi as u8;
    rom[vectors + 1] = (nmi >> 8) as u8;
    rom[vectors + 2] = 0x00; // reset -> $8000
    rom[vectors + 3] = 0x80;
    rom
}

/// Write PPU address then data bytes through $2006/$2007.
fn write_ppu(asm: &mut Asm, address: u16, bytes: &[u8]) {
    asm.lda_imm((address >> 8) as u8);
    asm.sta_abs(0x2006);
    asm.lda_imm(address as u8);
    asm.sta_abs(0x2006);
    for &byte in bytes {
        asm.lda_imm(byte);
        asm.sta_abs(0x2007);
    }
}

const FRAME_BUDGET: u32 = 200_000;

#[test]
fn ines_rejects_bad_media() {
    assert_eq!(Nes::new(b"garbage").err(), Some(CartridgeError::NotInes));
    let mut mapper1 = b"NES\x1A".to_vec();
    mapper1.extend_from_slice(&[1, 1, 0x10, 0]);
    mapper1.extend_from_slice(&[0; 8]);
    mapper1.resize(16 + 0x4000 + 0x2000, 0);
    assert_eq!(
        Nes::new(&mapper1).err(),
        Some(CartridgeError::UnsupportedMapper(1))
    );
}

#[test]
fn nmi_reaches_the_handler_once_per_frame() {
    let mut asm = Asm::new();
    asm.lda_imm(0x00);
    asm.sta_zp(0x10);
    asm.lda_imm(0x80); // NMI on
    asm.sta_abs(0x2000);
    let spin = asm.here();
    asm.jmp(spin);
    let nmi = asm.here();
    asm.inc_zp(0x10);
    asm.rti();

    let mut nes = Nes::new(&finish_rom(asm, nmi, &[])).unwrap();
    for _ in 0..3 {
        let _ = nes.step_frame(FRAME_BUDGET).expect("frame");
    }
    let count = nes.peek(0x0010);
    assert!((2..=3).contains(&count), "one NMI per frame, got {count}");
}

/// A solid-colour-3 tile in CHR (both planes set).
fn solid_tile() -> Vec<u8> {
    let mut chr = vec![0u8; 32];
    for byte in chr[16..32].iter_mut() {
        *byte = 0xFF; // tile 1: plane 0 and 1 all set
    }
    chr
}

#[test]
fn background_tile_renders_through_the_palette() {
    let mut asm = Asm::new();
    // Palette: backdrop $0F, BG palette 0 colour 3 = $21.
    write_ppu(&mut asm, 0x3F00, &[0x0F, 0x01, 0x02, 0x21]);
    // Nametable (0,0) = tile 1.
    write_ppu(&mut asm, 0x2000, &[0x01]);
    asm.lda_imm(0x00);
    asm.sta_abs(0x2005);
    asm.sta_abs(0x2005);
    asm.sta_abs(0x2006);
    asm.sta_abs(0x2006);
    asm.lda_imm(0x0A); // BG on, no left clip
    asm.sta_abs(0x2001);
    let spin = asm.here();
    asm.jmp(spin);

    let mut nes = Nes::new(&finish_rom(asm, 0x8000, &solid_tile())).unwrap();
    let _ = nes.step_frame(FRAME_BUDGET).expect("first frame");
    let frame = nes.step_frame(FRAME_BUDGET).expect("second frame");
    assert_eq!(frame.pixels.len(), 256 * ppu::VISIBLE_LINES as usize);
    assert!(
        frame.pixels[..8].iter().all(|&p| p == 0x21),
        "tile colour through palette, got {:?}",
        &frame.pixels[..10]
    );
    assert_eq!(frame.pixels[8], 0x0F, "backdrop after the tile");
}

#[test]
fn oam_dma_copies_a_page_and_sprites_render() {
    let mut asm = Asm::new();
    write_ppu(&mut asm, 0x3F00, &[0x0F]);
    // Sprite palette 0 colour 3 = $30.
    write_ppu(&mut asm, 0x3F10, &[0x0F, 0x04, 0x05, 0x30]);
    // Sprite 0 in RAM page 2: y=40, tile 1, attributes 0, x=60.
    asm.lda_imm(40);
    asm.sta_abs(0x0200);
    asm.lda_imm(1);
    asm.sta_abs(0x0201);
    asm.lda_imm(0);
    asm.sta_abs(0x0202);
    asm.lda_imm(60);
    asm.sta_abs(0x0203);
    // Park the rest of the page off-screen.
    asm.lda_imm(0xF0);
    asm.ldx_imm(4);
    let park = asm.here();
    asm.emit(&[0x9D, 0x00, 0x02]); // sta $0200,x
    asm.inx();
    asm.cpx_imm(0xFF);
    asm.bne_to(park);
    asm.lda_imm(0x02);
    asm.sta_abs(0x4014); // OAM DMA
    asm.lda_imm(0x12); // sprites on, no left clip
    asm.sta_abs(0x2001);
    let spin = asm.here();
    asm.jmp(spin);

    let mut nes = Nes::new(&finish_rom(asm, 0x8000, &solid_tile())).unwrap();
    let _ = nes.step_frame(FRAME_BUDGET).expect("first frame");
    let frame = nes.step_frame(FRAME_BUDGET).expect("second frame");
    assert_eq!(nes.ppu.oam[0], 40, "DMA landed in OAM");
    let line = 41usize; // sprite y + 1
    let row = &frame.pixels[line * 256..line * 256 + 256];
    assert!(
        row[60..68].iter().all(|&p| p == 0x30),
        "sprite colour, got {:?}",
        &row[58..70]
    );
}

#[test]
fn controller_shifts_the_button_order() {
    let mut asm = Asm::new();
    // Strobe, then shift 8 reads into $20 (bit 0 arrives first).
    asm.lda_imm(1);
    asm.sta_abs(0x4016);
    asm.lda_imm(0);
    asm.sta_abs(0x4016);
    asm.ldx_imm(8);
    let read = asm.here();
    asm.lda_abs(0x4016);
    asm.lsr_a();
    asm.rol_zp(0x20);
    asm.emit(&[0xCA]); // dex
    asm.bne_to(read);
    let spin = asm.here();
    asm.jmp(spin);

    let mut nes = Nes::new(&finish_rom(asm, 0x8000, &[])).unwrap();
    // A + Up pressed: bits 0 and 4 of the shift order.
    nes.set_controller(0b0001_0001);
    for _ in 0..2000 {
        nes.step_instruction();
    }
    // rol accumulates first-read into bit 7: A,B,Sel,St,U,D,L,R top-down.
    assert_eq!(nes.peek(0x0020), 0b1000_1000);
}
