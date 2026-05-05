use crate::common;
use missingno_gb::GameBoy;

const TIMEOUT_FRAMES: u32 = 7200; // 120 seconds at ~60fps

// Decodes the wilbertpol mooneye fork's `Runtime-State` ramsection — `regs_save`
// (actual values), `regs_flags` (bit-per-assertion), `regs_assert` (expected values).
// Layout is: 8 + 1 + 8 = 17 bytes at WRAM slot 2 base. Base is wlalink's convention
// for unpositioned ramsections, not a source-pinned address.
const RECORD_BASE: u16 = 0xC000;
const RECORD_LEN: u16 = 17;

#[derive(Clone, Copy)]
enum AssertReg {
    A,
    F,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl AssertReg {
    const ITER: [AssertReg; 8] = [
        AssertReg::A,
        AssertReg::F,
        AssertReg::B,
        AssertReg::C,
        AssertReg::D,
        AssertReg::E,
        AssertReg::H,
        AssertReg::L,
    ];

    fn flag_bit(self) -> u8 {
        match self {
            AssertReg::A => 0,
            AssertReg::F => 1,
            AssertReg::B => 2,
            AssertReg::C => 3,
            AssertReg::D => 4,
            AssertReg::E => 5,
            AssertReg::H => 6,
            AssertReg::L => 7,
        }
    }

    // Byte offset within `reg_dump`. Order is `f, a, c, b, e, d, l, h`, matching
    // `push hl; push de; push bc; push af` from `regs_save + 8` (low to high).
    fn dump_offset(self) -> usize {
        match self {
            AssertReg::F => 0,
            AssertReg::A => 1,
            AssertReg::C => 2,
            AssertReg::B => 3,
            AssertReg::E => 4,
            AssertReg::D => 5,
            AssertReg::L => 6,
            AssertReg::H => 7,
        }
    }

    fn label(self) -> &'static str {
        match self {
            AssertReg::A => "a",
            AssertReg::F => "f",
            AssertReg::B => "b",
            AssertReg::C => "c",
            AssertReg::D => "d",
            AssertReg::E => "e",
            AssertReg::H => "h",
            AssertReg::L => "l",
        }
    }
}

struct FailedAssertion {
    reg: AssertReg,
    expected: u8,
    actual: u8,
}

fn decode_assertion_record(gb: &GameBoy) -> (u8, Vec<FailedAssertion>) {
    let bytes = gb.peek_range(RECORD_BASE, RECORD_LEN);
    let save = &bytes[0..8];
    let flags = bytes[8];
    let assert = &bytes[9..17];

    let mut failed = Vec::new();
    for reg in AssertReg::ITER {
        if flags & (1 << reg.flag_bit()) == 0 {
            continue;
        }
        let off = reg.dump_offset();
        if save[off] != assert[off] {
            failed.push(FailedAssertion {
                reg,
                expected: assert[off],
                actual: save[off],
            });
        }
    }
    (flags, failed)
}

fn run_wilbertpol_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    // Wilbertpol tests use 0xED (undefined opcode) as their exit condition
    let found = common::run_until_undefined_opcode(&mut run, TIMEOUT_FRAMES);
    assert!(
        found,
        "Mooneye-wilbertpol test {rom_path} timed out without reaching exit condition"
    );
    let cpu = run.gb.cpu();
    if common::check_mooneye_pass(cpu) {
        return;
    }

    let (flags, failed) = decode_assertion_record(&run.gb);
    if !failed.is_empty() {
        let mut msg = format!(
            "Mooneye-wilbertpol test {rom_path} failed: {} assertion(s)",
            failed.len()
        );
        for f in &failed {
            msg.push_str(&format!(
                "\n  assert_{}: expected 0x{:02X}, got 0x{:02X}",
                f.reg.label(),
                f.expected,
                f.actual,
            ));
        }
        panic!("{msg}");
    }

    panic!(
        "Mooneye-wilbertpol test {rom_path} failed with no per-assertion mismatch \
         (regs_flags=0x{flags:02X}; either no asserts were registered, or all set asserts matched). \
         Post-halt registers: A=0x{:02X} B=0x{:02X} C=0x{:02X} D=0x{:02X} E=0x{:02X} H=0x{:02X} L=0x{:02X}",
        cpu.a, cpu.b, cpu.c, cpu.d, cpu.e, cpu.h, cpu.l,
    );
}

macro_rules! wilbertpol_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_wilbertpol_test($path);
        }
    };
}

// acceptance/gpu/ — PPU timing tests (unique to wilbertpol fork)

wilbertpol_test!(
    gpu_hblank_ly_scx_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/hblank_ly_scx_timing_nops.gb"
);
wilbertpol_test!(
    gpu_hblank_ly_scx_timing_variant_nops,
    "mooneye-wilbertpol/acceptance/gpu/hblank_ly_scx_timing_variant_nops.gb"
);
wilbertpol_test!(
    gpu_intr_0_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_0_timing.gb"
);
wilbertpol_test!(
    gpu_intr_1_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_1_timing.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx1_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx1_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx2_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx2_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx3_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx3_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx4_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx4_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx5_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx5_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx6_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx6_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx7_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx7_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_scx8_timing_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_scx8_timing_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites_scx1_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_scx1_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites_scx2_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_scx2_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites_scx3_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_scx3_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites_scx4_nops,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_scx4_nops.gb"
);
wilbertpol_test!(
    gpu_intr_2_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_timing.gb"
);
wilbertpol_test!(
    gpu_lcdon_mode_timing,
    "mooneye-wilbertpol/acceptance/gpu/lcdon_mode_timing.gb"
);
wilbertpol_test!(
    gpu_ly00_01_mode0_2,
    "mooneye-wilbertpol/acceptance/gpu/ly00_01_mode0_2.gb"
);
wilbertpol_test!(
    gpu_ly00_mode0_2_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly00_mode0_2-GS.gb"
);
wilbertpol_test!(
    gpu_ly00_mode1_0_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly00_mode1_0-GS.gb"
);
wilbertpol_test!(
    gpu_ly00_mode2_3,
    "mooneye-wilbertpol/acceptance/gpu/ly00_mode2_3.gb"
);
wilbertpol_test!(
    gpu_ly00_mode3_0,
    "mooneye-wilbertpol/acceptance/gpu/ly00_mode3_0.gb"
);
wilbertpol_test!(
    gpu_ly143_144_145,
    "mooneye-wilbertpol/acceptance/gpu/ly143_144_145.gb"
);
wilbertpol_test!(
    gpu_ly143_144_152_153,
    "mooneye-wilbertpol/acceptance/gpu/ly143_144_152_153.gb"
);
wilbertpol_test!(
    gpu_ly143_144_mode0_1,
    "mooneye-wilbertpol/acceptance/gpu/ly143_144_mode0_1.gb"
);
wilbertpol_test!(
    gpu_ly143_144_mode3_0,
    "mooneye-wilbertpol/acceptance/gpu/ly143_144_mode3_0.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_0_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_0-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_0_write_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_0_write-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_144_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_144-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_153_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_153-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_153_write_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_153_write-GS.gb"
);
wilbertpol_test!(
    gpu_ly_lyc_write_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_lyc_write-GS.gb"
);
wilbertpol_test!(
    gpu_ly_new_frame_gs,
    "mooneye-wilbertpol/acceptance/gpu/ly_new_frame-GS.gb"
);
wilbertpol_test!(
    gpu_stat_irq_blocking,
    "mooneye-wilbertpol/acceptance/gpu/stat_irq_blocking.gb"
);
wilbertpol_test!(
    gpu_stat_write_if_gs,
    "mooneye-wilbertpol/acceptance/gpu/stat_write_if-GS.gb"
);
wilbertpol_test!(
    gpu_vblank_if_timing,
    "mooneye-wilbertpol/acceptance/gpu/vblank_if_timing.gb"
);

// acceptance/timer/
wilbertpol_test!(timer_if, "mooneye-wilbertpol/acceptance/timer/timer_if.gb");

// emulator-only/ — MBC tests
wilbertpol_test!(
    mbc1_rom_4banks,
    "mooneye-wilbertpol/emulator-only/mbc1_rom_4banks.gb"
);
