//! Mooneye Test Suite (wilbertpol fork) — CGB-compatible subset.
//!
//! Mirrors the existing curated 41-test DMG subset from the gb crate;
//! ROMs are shared via `common::load_rom`. The wilbertpol exit
//! condition is opcode `0xED` (undefined) instead of `LD B,B`; the
//! assertion record is decoded from WRAM after the test halts.
//!
//! Adding the full v7 wilbertpol set (~121 ROMs total, plus extra `-C`
//! CGB-specific tests) is mechanical follow-up work.

use crate::common;
use crate::common::System;

const TIMEOUT_FRAMES: u32 = 7200;

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

    fn dump_offset(self) -> usize {
        match self {
            AssertReg::A => 1,
            AssertReg::F => 0,
            AssertReg::B => 3,
            AssertReg::C => 2,
            AssertReg::D => 5,
            AssertReg::E => 4,
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

fn decode_assertion_record<S: System>(s: &S) -> (u8, Vec<FailedAssertion>) {
    let bytes = s.peek_range(RECORD_BASE, RECORD_LEN);
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
    let mut gbc = common::load_rom(rom_path);
    let found = common::run_until_undefined_opcode(&mut gbc, TIMEOUT_FRAMES);
    assert!(
        found,
        "Mooneye-wilbertpol test {rom_path} timed out without reaching exit condition"
    );
    let cpu = gbc.cpu();
    if common::check_mooneye_pass(cpu) {
        return;
    }

    let (flags, failed) = decode_assertion_record(&gbc);
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

    let testcase_id = gbc.peek_range(0xC000, 1)[0];
    panic!(
        "Mooneye-wilbertpol test {rom_path} failed with no per-assertion mismatch \
         (regs_flags=0x{flags:02X}). testcase_id=0x{testcase_id:02X}. \
         Registers: {}",
        common::format_registers(cpu),
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
    gpu_stat_irq_blocking,
    "mooneye-wilbertpol/acceptance/gpu/stat_irq_blocking.gb"
);
wilbertpol_test!(
    gpu_vblank_if_timing,
    "mooneye-wilbertpol/acceptance/gpu/vblank_if_timing.gb"
);
wilbertpol_test!(timer_if, "mooneye-wilbertpol/acceptance/timer/timer_if.gb");
wilbertpol_test!(
    mbc1_rom_4banks,
    "mooneye-wilbertpol/emulator-only/mbc1_rom_4banks.gb"
);

// ── Newly imported no-suffix wilbertpol tests (DMG+CGB compat, shared ROMs)
wilbertpol_test!(add_sp_e_timing, "mooneye-wilbertpol/acceptance/add_sp_e_timing.gb");
wilbertpol_test!(call_cc_timing2, "mooneye-wilbertpol/acceptance/call_cc_timing2.gb");
wilbertpol_test!(call_cc_timing, "mooneye-wilbertpol/acceptance/call_cc_timing.gb");
wilbertpol_test!(call_timing2, "mooneye-wilbertpol/acceptance/call_timing2.gb");
wilbertpol_test!(call_timing, "mooneye-wilbertpol/acceptance/call_timing.gb");
wilbertpol_test!(div_timing, "mooneye-wilbertpol/acceptance/div_timing.gb");
wilbertpol_test!(timer_div_write, "mooneye-wilbertpol/acceptance/timer/div_write.gb");
wilbertpol_test!(ei_timing, "mooneye-wilbertpol/acceptance/ei_timing.gb");
wilbertpol_test!(halt_ime0_ei, "mooneye-wilbertpol/acceptance/halt_ime0_ei.gb");
wilbertpol_test!(halt_ime0_nointr_timing, "mooneye-wilbertpol/acceptance/halt_ime0_nointr_timing.gb");
wilbertpol_test!(halt_ime1_timing, "mooneye-wilbertpol/acceptance/halt_ime1_timing.gb");
wilbertpol_test!(if_ie_registers, "mooneye-wilbertpol/acceptance/if_ie_registers.gb");
wilbertpol_test!(gpu_intr_2_0_timing, "mooneye-wilbertpol/acceptance/gpu/intr_2_0_timing.gb");
wilbertpol_test!(gpu_intr_2_mode0_timing, "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing.gb");
wilbertpol_test!(gpu_intr_2_mode0_timing_sprites, "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites.gb");
wilbertpol_test!(gpu_intr_2_mode3_timing, "mooneye-wilbertpol/acceptance/gpu/intr_2_mode3_timing.gb");
wilbertpol_test!(gpu_intr_2_oam_ok_timing, "mooneye-wilbertpol/acceptance/gpu/intr_2_oam_ok_timing.gb");
wilbertpol_test!(intr_timing, "mooneye-wilbertpol/acceptance/intr_timing.gb");
wilbertpol_test!(jp_cc_timing, "mooneye-wilbertpol/acceptance/jp_cc_timing.gb");
wilbertpol_test!(jp_timing, "mooneye-wilbertpol/acceptance/jp_timing.gb");
wilbertpol_test!(ld_hl_sp_e_timing, "mooneye-wilbertpol/acceptance/ld_hl_sp_e_timing.gb");
wilbertpol_test!(bits_mem_oam, "mooneye-wilbertpol/acceptance/bits/mem_oam.gb");
wilbertpol_test!(oam_dma_restart, "mooneye-wilbertpol/acceptance/oam_dma_restart.gb");
wilbertpol_test!(oam_dma_start, "mooneye-wilbertpol/acceptance/oam_dma_start.gb");
wilbertpol_test!(oam_dma_timing, "mooneye-wilbertpol/acceptance/oam_dma_timing.gb");
wilbertpol_test!(pop_timing, "mooneye-wilbertpol/acceptance/pop_timing.gb");
wilbertpol_test!(push_timing, "mooneye-wilbertpol/acceptance/push_timing.gb");
wilbertpol_test!(rapid_di_ei, "mooneye-wilbertpol/acceptance/rapid_di_ei.gb");
wilbertpol_test!(timer_rapid_toggle, "mooneye-wilbertpol/acceptance/timer/rapid_toggle.gb");
wilbertpol_test!(bits_reg_f, "mooneye-wilbertpol/acceptance/bits/reg_f.gb");
wilbertpol_test!(ret_cc_timing, "mooneye-wilbertpol/acceptance/ret_cc_timing.gb");
wilbertpol_test!(reti_intr_timing, "mooneye-wilbertpol/acceptance/reti_intr_timing.gb");
wilbertpol_test!(reti_timing, "mooneye-wilbertpol/acceptance/reti_timing.gb");
wilbertpol_test!(ret_timing, "mooneye-wilbertpol/acceptance/ret_timing.gb");
wilbertpol_test!(rst_timing, "mooneye-wilbertpol/acceptance/rst_timing.gb");
wilbertpol_test!(timer_tim00_div_trigger, "mooneye-wilbertpol/acceptance/timer/tim00_div_trigger.gb");
wilbertpol_test!(timer_tim00, "mooneye-wilbertpol/acceptance/timer/tim00.gb");
wilbertpol_test!(timer_tim01_div_trigger, "mooneye-wilbertpol/acceptance/timer/tim01_div_trigger.gb");
wilbertpol_test!(timer_tim01, "mooneye-wilbertpol/acceptance/timer/tim01.gb");
wilbertpol_test!(timer_tim10_div_trigger, "mooneye-wilbertpol/acceptance/timer/tim10_div_trigger.gb");
wilbertpol_test!(timer_tim10, "mooneye-wilbertpol/acceptance/timer/tim10.gb");
wilbertpol_test!(timer_tim11_div_trigger, "mooneye-wilbertpol/acceptance/timer/tim11_div_trigger.gb");
wilbertpol_test!(timer_tim11, "mooneye-wilbertpol/acceptance/timer/tim11.gb");
wilbertpol_test!(timer_tima_reload, "mooneye-wilbertpol/acceptance/timer/tima_reload.gb");
wilbertpol_test!(timer_tima_write_reloading, "mooneye-wilbertpol/acceptance/timer/tima_write_reloading.gb");
wilbertpol_test!(timer_tma_write_reloading, "mooneye-wilbertpol/acceptance/timer/tma_write_reloading.gb");

// ── CGB-only wilbertpol tests (`-C` and `-cgb` suffixes; ROMs in gbc crate)

fn run_wilbertpol_cgb_test(rom_path: &str) {
    let mut gbc = common::load_cgb_rom(rom_path);
    let found = common::run_until_undefined_opcode(&mut gbc, TIMEOUT_FRAMES);
    assert!(
        found,
        "Mooneye-wilbertpol test {rom_path} timed out without reaching exit condition"
    );
    let cpu = gbc.cpu();
    if common::check_mooneye_pass(cpu) {
        return;
    }

    let (flags, failed) = decode_assertion_record(&gbc);
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

    let testcase_id = gbc.peek_range(0xC000, 1)[0];
    panic!(
        "Mooneye-wilbertpol test {rom_path} failed with no per-assertion mismatch \
         (regs_flags=0x{flags:02X}). testcase_id=0x{testcase_id:02X}. \
         Registers: {}",
        common::format_registers(cpu),
    );
}

macro_rules! wilbertpol_cgb_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_wilbertpol_cgb_test($path);
        }
    };
}

wilbertpol_cgb_test!(misc_boot_hwio_c, "mooneye-wilbertpol/misc/boot_hwio-C.gb");
wilbertpol_cgb_test!(misc_boot_regs_cgb, "mooneye-wilbertpol/misc/boot_regs-cgb.gb");
wilbertpol_cgb_test!(misc_bits_unused_hwio_c, "mooneye-wilbertpol/misc/bits/unused_hwio-C.gb");
wilbertpol_cgb_test!(misc_gpu_vblank_stat_intr_c, "mooneye-wilbertpol/misc/gpu/vblank_stat_intr-C.gb");
wilbertpol_cgb_test!(gpu_hblank_ly_scx_timing_c, "mooneye-wilbertpol/acceptance/gpu/hblank_ly_scx_timing-C.gb");
wilbertpol_cgb_test!(gpu_ly00_mode1_2_c, "mooneye-wilbertpol/acceptance/gpu/ly00_mode1_2-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_0_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_0-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_0_write_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_0_write-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_144_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_144-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_153_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_153-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_153_write_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_153_write-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc-C.gb");
wilbertpol_cgb_test!(gpu_ly_lyc_write_c, "mooneye-wilbertpol/acceptance/gpu/ly_lyc_write-C.gb");
wilbertpol_cgb_test!(gpu_ly_new_frame_c, "mooneye-wilbertpol/acceptance/gpu/ly_new_frame-C.gb");
wilbertpol_cgb_test!(gpu_stat_write_if_c, "mooneye-wilbertpol/acceptance/gpu/stat_write_if-C.gb");
