//! TIA object corpus: positioning, copies, motion, collisions, delay
//! latches. Kernels avoid absolute line counting — the free-running
//! counters repeat an object on every line, so tests mark phases by
//! switching object colours and scan the frame for them.

use missingno_vcs::console::Vcs;

const VSYNC: u8 = 0x00;
const WSYNC: u8 = 0x02;
const NUSIZ0: u8 = 0x04;
const COLUP0: u8 = 0x06;
const COLUBK: u8 = 0x09;
const PF0: u8 = 0x0D;
const RESP0: u8 = 0x10;
const GRP0: u8 = 0x1B;
const GRP1: u8 = 0x1C;
const HMP0: u8 = 0x20;
const HMOVE: u8 = 0x2A;
const CXCLR: u8 = 0x2C;
const CXP0FB: u8 = 0x02;

const COLOR_A: u8 = 0x1E;
const COLOR_B: u8 = 0x5E;

struct Asm {
    origin: u16,
    bytes: Vec<u8>,
}

impl Asm {
    fn new() -> Self {
        Asm {
            origin: 0xF000,
            bytes: Vec::new(),
        }
    }
    fn here(&self) -> u16 {
        self.origin + self.bytes.len() as u16
    }
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
    fn lda_imm(&mut self, v: u8) {
        self.emit(&[0xA9, v]);
    }
    fn ldx_imm(&mut self, v: u8) {
        self.emit(&[0xA2, v]);
    }
    fn sta_zp(&mut self, a: u8) {
        self.emit(&[0x85, a]);
    }
    fn lda_zp(&mut self, a: u8) {
        self.emit(&[0xA5, a]);
    }
    fn nop(&mut self) {
        self.emit(&[0xEA]);
    }
    fn dex(&mut self) {
        self.emit(&[0xCA]);
    }
    fn bne_to(&mut self, target: u16) {
        let offset = target as i32 - (self.here() as i32 + 2);
        self.emit(&[0xD0, i8::try_from(offset).unwrap() as u8]);
    }
    fn jmp_abs(&mut self, target: u16) {
        self.emit(&[0x4C, target as u8, (target >> 8) as u8]);
    }
    fn into_rom(self) -> Vec<u8> {
        let mut rom = self.bytes;
        rom.resize(0x1000, 0);
        rom[0xFFC] = self.origin as u8;
        rom[0xFFD] = (self.origin >> 8) as u8;
        rom
    }
}

/// Frame skeleton: VSYNC, a body that runs once per frame starting at a
/// line boundary, then padding lines and the loop.
fn kernel(setup: impl Fn(&mut Asm), body: impl Fn(&mut Asm)) -> Vec<u8> {
    let mut asm = Asm::new();
    setup(&mut asm);

    let frame = asm.here();
    asm.lda_imm(0x02);
    asm.sta_zp(VSYNC);
    for _ in 0..3 {
        asm.sta_zp(WSYNC);
    }
    asm.lda_imm(0x00);
    asm.sta_zp(VSYNC);
    asm.sta_zp(WSYNC);
    body(&mut asm);
    asm.sta_zp(WSYNC);
    asm.ldx_imm(40);
    let pad = asm.here();
    asm.sta_zp(WSYNC);
    asm.dex();
    asm.bne_to(pad);
    asm.jmp_abs(frame);
    asm.into_rom()
}

fn second_frame(rom: &[u8]) -> Vec<[u8; 160]> {
    let mut vcs = Vcs::new(rom).unwrap();
    let _ = vcs.step_frame(1000).expect("first frame");
    vcs.step_frame(1000).expect("second frame").lines
}

fn lit_pixels(line: &[u8; 160], color: u8) -> Vec<usize> {
    line.iter()
        .enumerate()
        .filter(|&(_, &p)| p == color & 0xFE)
        .map(|(x, _)| x)
        .collect()
}

/// First line drawn in `color`, with its lit pixel positions.
fn find_color(lines: &[[u8; 160]], color: u8) -> Option<(usize, Vec<usize>)> {
    find_color_after(lines, color, 0)
}

/// Frame state persists through the loop, so "after" phases of a kernel
/// must be searched after the "before" line, not from the frame top.
fn find_color_after(lines: &[[u8; 160]], color: u8, start: usize) -> Option<(usize, Vec<usize>)> {
    lines.iter().enumerate().skip(start).find_map(|(i, line)| {
        let lit = lit_pixels(line, color);
        (!lit.is_empty()).then_some((i, lit))
    })
}

#[test]
fn resp_during_hblank_parks_at_left_edge() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
        },
        |asm| {
            asm.sta_zp(RESP0); // write cycle lands at colour clock 6: hblank
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("player drawn");
    assert_eq!(lit.len(), 8, "8-pixel player, got {lit:?}");
    assert_eq!(lit[0], 3, "hblank RESP parks the player at x=3");
}

#[test]
fn nusiz_three_copies_close() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.lda_imm(0x03);
            asm.sta_zp(NUSIZ0);
        },
        |asm| {
            asm.sta_zp(RESP0);
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("copies drawn");
    assert_eq!(lit.len(), 24, "three 8-pixel copies, got {lit:?}");
    assert_eq!(lit[0], 3);
    assert_eq!(lit[8], 3 + 16, "close copy at +16");
    assert_eq!(lit[16], 3 + 32, "second close copy at +32");
}

/// RESP mid-line: the player lands strobe-position + 5 (the documented
/// mid-line landing: write cycle's clock + the start pipeline).
#[test]
fn resp_mid_line_lands_at_strobe_plus_five() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
        },
        |asm| {
            // 25 NOPs = 50 cycles; STA RESP0's write cycle is cycle 52
            // of the line = colour clock 156... wraps? No: 52*3 = 156,
            // still inside the 228-clock line, visible x = 156-68 = 88.
            for _ in 0..25 {
                asm.nop();
            }
            asm.sta_zp(RESP0);
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("player drawn");
    assert_eq!(lit.len(), 8, "8-pixel player, got {lit:?}");
    // Strobe at visible x=88; hardware lands the player at x+5.
    assert_eq!(lit[0], 88 + 5, "mid-line RESP lands strobe+5, got {lit:?}");
}

/// HMOVE +7 after positioning: colour A marks pre-move lines, colour B
/// post-move; the move shifts 7 left and the comb blanks 8 pixels.
#[test]
fn hmove_moves_left_and_blanks_the_comb() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(0x0E);
            asm.sta_zp(COLUBK);
        },
        |asm| {
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            // Park mid-line (x=93) so a 7-left move stays unwrapped.
            for _ in 0..25 {
                asm.nop();
            }
            asm.sta_zp(RESP0);
            asm.sta_zp(WSYNC);
            asm.lda_imm(0x70);
            asm.sta_zp(HMP0);
            asm.sta_zp(WSYNC);
            asm.sta_zp(HMOVE); // strobed at line start: comb + move
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP0); // recoloured within the comb
        },
    );
    let lines = second_frame(&rom);
    let (line_a, _) = find_color(&lines, COLOR_A).expect("pre-move player");
    let (line_b, lit_b) = find_color_after(&lines, COLOR_B, line_a + 1).expect("post-move player");
    // The strobe line can merge stale-phase and fresh draws; the settled
    // pre-move position is the line just before the HMOVE line.
    let lit_a = lit_pixels(&lines[line_b - 1], COLOR_A);
    assert_eq!(lit_a.len(), 8, "pre-move player, got {lit_a:?}");
    assert_eq!(lit_b.len(), 8, "post-move player, got {lit_b:?}");
    assert_eq!(
        lit_b[0] + 7,
        lit_a[0],
        "HMP0=+7 shifts 7 left (pre {lit_a:?}, post {lit_b:?})"
    );
    // The comb line: backdrop is lit, but its first 8 pixels are blanked.
    let comb = &lines[line_b];
    assert!(
        comb[..8].iter().all(|&p| p == 0),
        "comb blanks 8 pixels, got {:02X?}",
        &comb[..8]
    );
    assert_eq!(comb[8], 0x0E, "backdrop resumes after the comb");
}

/// Zero motion still combs: HMOVE with HMP0=0 must not move the player.
#[test]
fn hmove_zero_motion_holds_position() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
        },
        |asm| {
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.sta_zp(RESP0);
            asm.sta_zp(WSYNC);
            asm.lda_imm(0x00);
            asm.sta_zp(HMP0);
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP0);
            asm.sta_zp(WSYNC);
            asm.sta_zp(HMOVE);
        },
    );
    let lines = second_frame(&rom);
    let (line_a, lit_a) = find_color(&lines, COLOR_A).expect("pre player");
    let (_, lit_b) = find_color_after(&lines, COLOR_B, line_a + 1).expect("post player");
    assert_eq!(lit_a[0], lit_b[0], "HM=0: comb compensates the 8 clocks");
}

#[test]
fn player_playfield_collision_latches() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.sta_zp(PF0); // playfield across x=0..16, over the player
            asm.sta_zp(CXCLR);
        },
        |asm| {
            asm.sta_zp(RESP0);
            asm.sta_zp(WSYNC);
            asm.sta_zp(WSYNC);
            asm.lda_zp(CXP0FB);
            asm.sta_zp(0x80); // stash the latch in RIOT RAM
        },
    );
    let mut vcs = Vcs::new(&rom).unwrap();
    let _ = vcs.step_frame(1000).unwrap();
    let _ = vcs.step_frame(1000).unwrap();
    assert_eq!(
        vcs.riot.ram[0] & 0x80,
        0x80,
        "player-playfield collision latched"
    );
}

/// VDEL: while delayed, GRP0 shows its OLD register; the new value only
/// appears after a GRP1 write commits it. Colour A marks the pre-commit
/// lines — a correct latch never draws in colour A.
#[test]
fn vertical_delay_swaps_on_grp1_write() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0x01);
            asm.sta_zp(0x25); // VDELP0
        },
        |asm| {
            // Reset the latch pair each frame: old <- 0, new <- FF.
            asm.lda_imm(0x00);
            asm.sta_zp(GRP0);
            asm.sta_zp(GRP1); // commits old <- new (0)
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0); // new = FF, old still 0
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.sta_zp(RESP0);
            asm.sta_zp(WSYNC);
            asm.sta_zp(WSYNC);
            // Commit and recolour: visible lines are colour B only.
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP0);
            asm.lda_imm(0x00);
            asm.sta_zp(GRP1); // old <- FF
        },
    );
    let lines = second_frame(&rom);
    assert!(
        find_color(&lines, COLOR_A).is_none(),
        "delayed graphics must stay hidden before the GRP1 commit"
    );
    let (_, lit) = find_color(&lines, COLOR_B).expect("committed graphics visible");
    assert_eq!(lit.len(), 8);
}

const COLUP1: u8 = 0x07;
const ENAM0: u8 = 0x1D;
const ENABL: u8 = 0x1F;
const CTRLPF: u8 = 0x0A;
const RESM0: u8 = 0x12;
const RESBL: u8 = 0x14;
const REFP0: u8 = 0x0B;
const COLUPF: u8 = 0x08;
const PF1: u8 = 0x0E;

/// Missile: hblank RESM parks at x=2; NUSIZ bits 4-5 set the width.
#[test]
fn missile_width_and_landing() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0x02);
            asm.sta_zp(ENAM0);
            asm.lda_imm(0x20); // width 4
            asm.sta_zp(NUSIZ0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
        },
        |asm| {
            asm.sta_zp(RESM0);
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("missile drawn");
    assert_eq!(lit, vec![2, 3, 4, 5], "4-wide missile parked at x=2");
}

/// Ball: hblank RESBL parks at x=2; CTRLPF bits 4-5 set the width.
#[test]
fn ball_width_and_landing() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0x02);
            asm.sta_zp(ENABL);
            asm.lda_imm(0x20); // width 4
            asm.sta_zp(CTRLPF);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUPF); // ball draws in the playfield colour
        },
        |asm| {
            asm.sta_zp(RESBL);
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("ball drawn");
    assert_eq!(lit, vec![2, 3, 4, 5], "4-wide ball parked at x=2");
}

/// REFP: GRP0=0xC0 draws its two lit bits leading normally, trailing
/// when reflected.
#[test]
fn player_reflection_mirrors_the_bits() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xC0);
            asm.sta_zp(GRP0);
        },
        |asm| {
            asm.lda_imm(0x00);
            asm.sta_zp(REFP0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.sta_zp(RESP0);
            asm.sta_zp(WSYNC);
            asm.sta_zp(WSYNC);
            asm.lda_imm(0x08);
            asm.sta_zp(REFP0);
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP0);
        },
    );
    let lines = second_frame(&rom);
    let (line_a, _) = find_color(&lines, COLOR_A).expect("normal player");
    let (_, lit_b) = find_color_after(&lines, COLOR_B, line_a + 1).expect("reflected player");
    let lit_a = lit_pixels(&lines[line_a], COLOR_A);
    assert_eq!(lit_a, vec![3, 4], "bits 7-6 lead unreflected");
    assert_eq!(lit_b, vec![9, 10], "bits 7-6 trail reflected");
}

/// NUSIZ 7: quad-width player, 32 clocks wide.
#[test]
fn quad_player_stretches_to_32() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(0x07);
            asm.sta_zp(NUSIZ0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
        },
        |asm| {
            asm.sta_zp(RESP0);
        },
    );
    let (_, lit) = find_color(&second_frame(&rom), COLOR_A).expect("quad player");
    assert_eq!(lit.len(), 32, "8 bits at 4 clocks each, got {lit:?}");
    assert_eq!(lit[0], 3);
}

/// Score mode colours the left playfield half with COLUP0, the right
/// with COLUP1; priority mode puts the playfield above the players.
#[test]
fn score_and_priority_modes() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(PF1); // band at x=16..48 each half
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP1);
            asm.lda_imm(0x02); // score mode
            asm.sta_zp(CTRLPF);
        },
        |asm| {
            asm.nop();
        },
    );
    let lines = second_frame(&rom);
    let (_, left) = find_color(&lines, COLOR_A).expect("left half in COLUP0");
    assert_eq!(left, (16..48).collect::<Vec<_>>());
    let (_, right) = find_color(&lines, COLOR_B).expect("right half in COLUP1");
    assert_eq!(right, (96..128).collect::<Vec<_>>());

    // Priority: a player overlapping the playfield hides behind it.
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(PF0); // playfield over x=0..16
            asm.sta_zp(GRP0);
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            asm.lda_imm(0x4E);
            asm.sta_zp(COLUPF);
            asm.lda_imm(0x04); // playfield priority
            asm.sta_zp(CTRLPF);
        },
        |asm| {
            asm.sta_zp(RESP0); // player at x=3, fully under the playfield
        },
    );
    let lines = second_frame(&rom);
    assert!(
        find_color(&lines, COLOR_A).is_none(),
        "player hidden behind the prioritised playfield"
    );
}

/// A mid-line ("illegal") HMOVE delivers its extra clocks without the
/// hblank compensation: HM=+7 shifts a full 15 left. Characterises the
/// mechanism's emergent behaviour — the value the Cosmic Ark family of
/// tricks builds on.
#[test]
fn illegal_mid_line_hmove_shifts_uncompensated() {
    let rom = kernel(
        |asm| {
            asm.lda_imm(0xFF);
            asm.sta_zp(GRP0);
            asm.lda_imm(0x70);
            asm.sta_zp(HMP0);
        },
        |asm| {
            asm.lda_imm(COLOR_A);
            asm.sta_zp(COLUP0);
            for _ in 0..25 {
                asm.nop();
            }
            asm.sta_zp(RESP0); // parks at x=93
            asm.sta_zp(WSYNC);
            asm.sta_zp(WSYNC);
            asm.lda_imm(COLOR_B);
            asm.sta_zp(COLUP0);
            for _ in 0..20 {
                asm.nop(); // reach mid-visible before strobing
            }
            asm.sta_zp(HMOVE);
            asm.sta_zp(WSYNC);
        },
    );
    let lines = second_frame(&rom);
    let (line_a, _) = find_color(&lines, COLOR_A).expect("parked player");
    let (_, lit_b) = find_color_after(&lines, COLOR_B, line_a + 1).expect("moved player");
    let lit_a = lit_pixels(&lines[line_a + 1], COLOR_A);
    assert_eq!(lit_a.first(), Some(&108), "settled pre-move position");
    // Find the settled post-move line (skip the smeared strobe line).
    let settled_b = lines
        .iter()
        .rev()
        .map(|l| lit_pixels(l, COLOR_B))
        .find(|lit| lit.len() == 8)
        .expect("settled post-move line");
    assert_eq!(
        settled_b[0],
        108 - 15,
        "mid-line HMOVE: 15 extra clocks, no comb compensation (got {settled_b:?}, strobe-line {lit_b:?})"
    );
}
