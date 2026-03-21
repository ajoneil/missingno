use crate::common;

const TIMEOUT_FRAMES: u32 = 7200; // 120 seconds at ~60fps

fn run_wilbertpol_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    // Wilbertpol tests use 0xED (undefined opcode) as their exit condition
    let found = common::run_until_undefined_opcode(&mut run, TIMEOUT_FRAMES);
    assert!(
        found,
        "Mooneye-wilbertpol test {rom_path} timed out without reaching exit condition"
    );
    let cpu = run.gb.cpu();
    if !common::check_mooneye_pass(cpu) {
        let all_same =
            cpu.b == cpu.c && cpu.c == cpu.d && cpu.d == cpu.e && cpu.e == cpu.h && cpu.h == cpu.l;
        if all_same && cpu.b != 0 {
            panic!(
                "Mooneye-wilbertpol test {rom_path} failed (all registers = 0x{:02X}, \
                 uniform failure — sub-test number unknown)",
                cpu.b,
            );
        }

        let fib = [
            (cpu.b, 3, "B"),
            (cpu.c, 5, "C"),
            (cpu.d, 8, "D"),
            (cpu.e, 13, "E"),
            (cpu.h, 21, "H"),
            (cpu.l, 34, "L"),
        ];
        let passed = fib
            .iter()
            .take_while(|(val, expected, _)| val == expected)
            .count();
        let failed_reg = if passed < 6 { fib[passed].2 } else { "?" };
        let failed_val = if passed < 6 { fib[passed].0 } else { 0 };
        panic!(
            "Mooneye-wilbertpol test {rom_path} failed at sub-test {} \
             (register {failed_reg}=0x{failed_val:02X}, expected {}). Registers: {}",
            passed + 1,
            if passed < 6 { fib[passed].1 } else { 0 },
            common::format_registers(cpu),
        );
    }
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
