use crate::common;

const TIMEOUT_FRAMES: u32 = 7200; // 120 seconds at ~60fps

fn run_wilbertpol_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    common::assert_wilbertpol_verdict(&mut run, rom_path, TIMEOUT_FRAMES);
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
// testcase 85 (5×OAM X=7 + 5×OAM X=167, SCX=1) fails by ~2 dots — dmg-sim
// and missingno agree on Mode 3 length (~245 dots), including on the
// actual wilbertpol ROM (not just custom sweep ROMs); the test
// calibration expects ≤242 dots. A real DMG-CPU-08 unit does pass this
// test. Per Gekkio's hardware database the DMG-CPU-08 designation is a
// mainboard revision (glop-top SoC + blobbed RAM), and Gekkio's decap
// confirms the glop-top SoC die is the same DMG-CPU B silicon dmg-sim
// was derived from. The 2-dot gap is therefore between dmg-sim's
// gate-level model and real B silicon, not a silicon-revision difference
// or a missingno extraction issue. Likely sources: cumulative gate-delay
// annotation error, idealised external SRAM / pad modelling, or process
// variation that puts real-silicon transistor strengths on one side of a
// dot boundary that dmg-sim's typical-corner model puts on the other.
// Needs further hardware testing (ideally the same test on a QFP-80
// DMG-CPU B unit to separate die effects from mainboard effects).
#[test]
fn gpu_intr_2_mode0_timing_sprites_scx1_nops() {
    run_wilbertpol_test(
        "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites_scx1_nops.gb",
    );
}
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

// ── Newly imported no-suffix wilbertpol tests (DMG+CGB compat)
wilbertpol_test!(
    add_sp_e_timing,
    "mooneye-wilbertpol/acceptance/add_sp_e_timing.gb"
);
wilbertpol_test!(
    call_cc_timing2,
    "mooneye-wilbertpol/acceptance/call_cc_timing2.gb"
);
wilbertpol_test!(
    call_cc_timing,
    "mooneye-wilbertpol/acceptance/call_cc_timing.gb"
);
wilbertpol_test!(
    call_timing2,
    "mooneye-wilbertpol/acceptance/call_timing2.gb"
);
wilbertpol_test!(call_timing, "mooneye-wilbertpol/acceptance/call_timing.gb");
wilbertpol_test!(div_timing, "mooneye-wilbertpol/acceptance/div_timing.gb");
wilbertpol_test!(
    timer_div_write,
    "mooneye-wilbertpol/acceptance/timer/div_write.gb"
);
wilbertpol_test!(ei_timing, "mooneye-wilbertpol/acceptance/ei_timing.gb");
wilbertpol_test!(
    halt_ime0_ei,
    "mooneye-wilbertpol/acceptance/halt_ime0_ei.gb"
);
wilbertpol_test!(
    halt_ime0_nointr_timing,
    "mooneye-wilbertpol/acceptance/halt_ime0_nointr_timing.gb"
);
wilbertpol_test!(
    halt_ime1_timing,
    "mooneye-wilbertpol/acceptance/halt_ime1_timing.gb"
);
wilbertpol_test!(
    if_ie_registers,
    "mooneye-wilbertpol/acceptance/if_ie_registers.gb"
);
wilbertpol_test!(
    gpu_intr_2_0_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_0_timing.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode0_timing_sprites,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode0_timing_sprites.gb"
);
wilbertpol_test!(
    gpu_intr_2_mode3_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_mode3_timing.gb"
);
wilbertpol_test!(
    gpu_intr_2_oam_ok_timing,
    "mooneye-wilbertpol/acceptance/gpu/intr_2_oam_ok_timing.gb"
);
wilbertpol_test!(intr_timing, "mooneye-wilbertpol/acceptance/intr_timing.gb");
wilbertpol_test!(
    jp_cc_timing,
    "mooneye-wilbertpol/acceptance/jp_cc_timing.gb"
);
wilbertpol_test!(jp_timing, "mooneye-wilbertpol/acceptance/jp_timing.gb");
wilbertpol_test!(
    ld_hl_sp_e_timing,
    "mooneye-wilbertpol/acceptance/ld_hl_sp_e_timing.gb"
);
wilbertpol_test!(
    bits_mem_oam,
    "mooneye-wilbertpol/acceptance/bits/mem_oam.gb"
);
wilbertpol_test!(
    oam_dma_restart,
    "mooneye-wilbertpol/acceptance/oam_dma_restart.gb"
);
wilbertpol_test!(
    oam_dma_start,
    "mooneye-wilbertpol/acceptance/oam_dma_start.gb"
);
wilbertpol_test!(
    oam_dma_timing,
    "mooneye-wilbertpol/acceptance/oam_dma_timing.gb"
);
wilbertpol_test!(pop_timing, "mooneye-wilbertpol/acceptance/pop_timing.gb");
wilbertpol_test!(push_timing, "mooneye-wilbertpol/acceptance/push_timing.gb");
wilbertpol_test!(rapid_di_ei, "mooneye-wilbertpol/acceptance/rapid_di_ei.gb");
wilbertpol_test!(
    timer_rapid_toggle,
    "mooneye-wilbertpol/acceptance/timer/rapid_toggle.gb"
);
wilbertpol_test!(bits_reg_f, "mooneye-wilbertpol/acceptance/bits/reg_f.gb");
wilbertpol_test!(
    ret_cc_timing,
    "mooneye-wilbertpol/acceptance/ret_cc_timing.gb"
);
wilbertpol_test!(
    reti_intr_timing,
    "mooneye-wilbertpol/acceptance/reti_intr_timing.gb"
);
wilbertpol_test!(reti_timing, "mooneye-wilbertpol/acceptance/reti_timing.gb");
wilbertpol_test!(ret_timing, "mooneye-wilbertpol/acceptance/ret_timing.gb");
wilbertpol_test!(rst_timing, "mooneye-wilbertpol/acceptance/rst_timing.gb");
wilbertpol_test!(
    timer_tim00_div_trigger,
    "mooneye-wilbertpol/acceptance/timer/tim00_div_trigger.gb"
);
wilbertpol_test!(timer_tim00, "mooneye-wilbertpol/acceptance/timer/tim00.gb");
wilbertpol_test!(
    timer_tim01_div_trigger,
    "mooneye-wilbertpol/acceptance/timer/tim01_div_trigger.gb"
);
wilbertpol_test!(timer_tim01, "mooneye-wilbertpol/acceptance/timer/tim01.gb");
wilbertpol_test!(
    timer_tim10_div_trigger,
    "mooneye-wilbertpol/acceptance/timer/tim10_div_trigger.gb"
);
wilbertpol_test!(timer_tim10, "mooneye-wilbertpol/acceptance/timer/tim10.gb");
wilbertpol_test!(
    timer_tim11_div_trigger,
    "mooneye-wilbertpol/acceptance/timer/tim11_div_trigger.gb"
);
wilbertpol_test!(timer_tim11, "mooneye-wilbertpol/acceptance/timer/tim11.gb");
wilbertpol_test!(
    timer_tima_reload,
    "mooneye-wilbertpol/acceptance/timer/tima_reload.gb"
);
wilbertpol_test!(
    timer_tima_write_reloading,
    "mooneye-wilbertpol/acceptance/timer/tima_write_reloading.gb"
);
wilbertpol_test!(
    timer_tma_write_reloading,
    "mooneye-wilbertpol/acceptance/timer/tma_write_reloading.gb"
);
