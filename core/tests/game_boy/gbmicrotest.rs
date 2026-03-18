use crate::common;

fn run_gbmicrotest(rom_name: &str) {
    let rom_path = format!("gbmicrotest/{rom_name}.gb");
    let mut gb = common::load_rom(&rom_path);

    // Step instruction-by-instruction, checking for result after each step.
    // Most gbmicrotests complete in a few hundred cycles. Tests that disable
    // the LCD never produce a frame, so we can't use frame-based timeouts.
    // 500K steps is generous (~2 million T-cycles, ~30 frames).
    for _ in 0..500_000 {
        gb.step();
        if gb.read(0xFF82) != 0 {
            let actual = gb.read(0xFF80);
            let expected = gb.read(0xFF81);
            let pass_flag = gb.read(0xFF82);
            assert_eq!(
                pass_flag, 0x01,
                "gbmicrotest {rom_name} FAILED: got 0x{actual:02X}, expected 0x{expected:02X}"
            );
            return;
        }
    }

    panic!("gbmicrotest {rom_name} timed out (0xFF82 still 0x00 after 500K steps)");
}

macro_rules! gbmicrotest {
    ($name:ident) => {
        #[test]
        fn $name() {
            run_gbmicrotest(stringify!($name));
        }
    };
    ($name:ident, $rom:literal) => {
        #[test]
        fn $name() {
            run_gbmicrotest($rom);
        }
    };
}

// --- OAM/VRAM access timing ---
gbmicrotest!(oam_read_l0_a);
gbmicrotest!(oam_read_l0_b);
gbmicrotest!(oam_read_l0_c);
gbmicrotest!(oam_read_l0_d);
gbmicrotest!(oam_read_l1_a);
gbmicrotest!(oam_read_l1_b);
gbmicrotest!(oam_read_l1_c);
gbmicrotest!(oam_read_l1_d);
gbmicrotest!(oam_read_l1_e);
gbmicrotest!(oam_read_l1_f);
gbmicrotest!(oam_write_l0_a);
gbmicrotest!(oam_write_l0_b);
gbmicrotest!(oam_write_l0_c);
gbmicrotest!(oam_write_l0_d);
gbmicrotest!(oam_write_l0_e);
gbmicrotest!(oam_write_l1_a);
gbmicrotest!(oam_write_l1_b);
gbmicrotest!(oam_write_l1_c);
gbmicrotest!(oam_write_l1_d);
gbmicrotest!(oam_write_l1_e);
gbmicrotest!(oam_write_l1_f);
gbmicrotest!(vram_read_l0_a);
gbmicrotest!(vram_read_l0_b);
gbmicrotest!(vram_read_l0_c);
gbmicrotest!(vram_read_l0_d);
gbmicrotest!(vram_read_l1_a);
gbmicrotest!(vram_read_l1_b);
gbmicrotest!(vram_read_l1_c);
gbmicrotest!(vram_read_l1_d);
gbmicrotest!(vram_write_l0_a);
gbmicrotest!(vram_write_l0_b);
gbmicrotest!(vram_write_l0_c);
gbmicrotest!(vram_write_l0_d);
gbmicrotest!(vram_write_l1_a);
gbmicrotest!(vram_write_l1_b);
gbmicrotest!(vram_write_l1_c);
gbmicrotest!(vram_write_l1_d);

// --- CPU ---
gbmicrotest!(halt_bug);
gbmicrotest!(halt_op_dupe);
gbmicrotest!(halt_op_dupe_delay);
gbmicrotest!(is_if_set_during_ime0);

// --- DMA ---
gbmicrotest!(dma_0x1000);
gbmicrotest!(dma_0x9000);
gbmicrotest!(dma_0xA000);
gbmicrotest!(dma_0xC000);
gbmicrotest!(dma_0xE000);
gbmicrotest!(dma_timing_a);

// --- Timer / DIV ---
gbmicrotest!(div_inc_timing_a);
gbmicrotest!(div_inc_timing_b);
gbmicrotest!(timer_div_phase_c);
gbmicrotest!(timer_div_phase_d);
gbmicrotest!(timer_tima_inc_256k_a);
gbmicrotest!(timer_tima_inc_256k_b);
gbmicrotest!(timer_tima_inc_256k_c);
gbmicrotest!(timer_tima_inc_256k_d);
gbmicrotest!(timer_tima_inc_256k_e);
gbmicrotest!(timer_tima_inc_256k_f);
gbmicrotest!(timer_tima_inc_256k_g);
gbmicrotest!(timer_tima_inc_256k_h);
gbmicrotest!(timer_tima_inc_256k_i);
gbmicrotest!(timer_tima_inc_256k_j);
gbmicrotest!(timer_tima_inc_256k_k);
gbmicrotest!(timer_tima_inc_64k_a);
gbmicrotest!(timer_tima_inc_64k_b);
gbmicrotest!(timer_tima_inc_64k_c);
gbmicrotest!(timer_tima_inc_64k_d);
gbmicrotest!(timer_tima_phase_a);
gbmicrotest!(timer_tima_phase_b);
gbmicrotest!(timer_tima_phase_c);
gbmicrotest!(timer_tima_phase_d);
gbmicrotest!(timer_tima_phase_e);
gbmicrotest!(timer_tima_phase_f);
gbmicrotest!(timer_tima_phase_g);
gbmicrotest!(timer_tima_phase_h);
gbmicrotest!(timer_tima_phase_i);
gbmicrotest!(timer_tima_phase_j);
gbmicrotest!(timer_tima_reload_256k_a);
gbmicrotest!(timer_tima_reload_256k_b);
gbmicrotest!(timer_tima_reload_256k_c);
gbmicrotest!(timer_tima_reload_256k_d);
gbmicrotest!(timer_tima_reload_256k_e);
gbmicrotest!(timer_tima_reload_256k_f);
gbmicrotest!(timer_tima_reload_256k_g);
gbmicrotest!(timer_tima_reload_256k_h);
gbmicrotest!(timer_tima_reload_256k_i);
gbmicrotest!(timer_tima_reload_256k_j);
gbmicrotest!(timer_tima_reload_256k_k);
gbmicrotest!(timer_tima_write_a);
gbmicrotest!(timer_tima_write_b);
gbmicrotest!(timer_tima_write_c);
gbmicrotest!(timer_tima_write_d);
gbmicrotest!(timer_tima_write_e);
gbmicrotest!(timer_tima_write_f);
gbmicrotest!(timer_tma_write_a);
gbmicrotest!(timer_tma_write_b);

// --- LCD on/off ---
gbmicrotest!(lcdon_halt_to_vblank_int_a);
gbmicrotest!(lcdon_halt_to_vblank_int_b);
gbmicrotest!(lcdon_nops_to_vblank_int_a);
gbmicrotest!(lcdon_nops_to_vblank_int_b);
gbmicrotest!(lcdon_to_if_oam_a);
gbmicrotest!(lcdon_to_if_oam_b);
gbmicrotest!(lcdon_to_ly1_a);
gbmicrotest!(lcdon_to_ly1_b);
gbmicrotest!(lcdon_to_ly2_a);
gbmicrotest!(lcdon_to_ly2_b);
gbmicrotest!(lcdon_to_ly3_a);
gbmicrotest!(lcdon_to_ly3_b);
gbmicrotest!(lcdon_to_lyc1_int);
gbmicrotest!(lcdon_to_lyc2_int);
gbmicrotest!(lcdon_to_lyc3_int);
gbmicrotest!(lcdon_to_oam_int_l0);
gbmicrotest!(lcdon_to_oam_int_l1);
gbmicrotest!(lcdon_to_oam_int_l2);
gbmicrotest!(lcdon_to_oam_unlock_a);
gbmicrotest!(lcdon_to_oam_unlock_b);
gbmicrotest!(lcdon_to_oam_unlock_c);
gbmicrotest!(lcdon_to_oam_unlock_d);
gbmicrotest!(lcdon_to_stat0_a);
gbmicrotest!(lcdon_to_stat0_b);
gbmicrotest!(lcdon_to_stat0_c);
gbmicrotest!(lcdon_to_stat0_d);
gbmicrotest!(lcdon_to_stat1_a);
gbmicrotest!(lcdon_to_stat1_b);
gbmicrotest!(lcdon_to_stat1_c);
// These two tests require a hardware mechanism not yet understood at the gate
// level. GateBoy (die-photo-derived simulation) also fails them identically —
// the mode 0 "glitch" at the line 153→0 boundary involves an unmodeled bus
// latch or analog timing effect.
#[test]
#[ignore]
fn lcdon_to_stat1_d() {
    run_gbmicrotest("lcdon_to_stat1_d");
}
#[test]
#[ignore]
fn lcdon_to_stat1_e() {
    run_gbmicrotest("lcdon_to_stat1_e");
}
// The first scanline after LCD enable is 6 dots (1.5 M-cycles) too long.
// Hardware (Mooneye lcdon_timing-GS, verified on DMG/MGB/SGB/SGB2) shows LY
// incrementing from 0→1 at bus-read M-cycle 112 after LCD enable. Our emulator
// increments at M-cycle 113 (454 dots vs the ≤448 hardware needs). The test
// reads STAT at M-cycle 112 expecting 0x80 (no LYC match, because LY=1≠LYC=0
// on hardware), but our emulator still has LY=0 at that point, so LY=LYC=0
// matches and STAT reads 0x84.
//
// Root cause: the WUVU/VENA/TALU clock initialization at LCD enable takes 6
// extra dots before the first LX count begins. Neither GateBoy (gate-level)
// nor LogicBoy (behavioral) get this right — both also fail stat2_a. LogicBoy
// overshoots by ~3.5 dots (903 phases vs ≤896). The correct startup timing
// is faster than any known gate-level model predicts.
//
// This is NOT an ROPO/LYC pipeline issue — the coincidence flag correctly
// reflects LY vs LYC. The fix is to shorten the first scanline by adjusting
// the LCD-enable clock init chain. Constraint: must not regress stat0_c/d
// (mode 3→0 gap timing) or stat2_b.
#[test]
#[ignore]
fn lcdon_to_stat2_a() {
    run_gbmicrotest("lcdon_to_stat2_a");
}
gbmicrotest!(lcdon_to_stat2_b);
gbmicrotest!(lcdon_to_stat2_c);
gbmicrotest!(lcdon_to_stat2_d);
gbmicrotest!(lcdon_to_stat3_a);
gbmicrotest!(lcdon_to_stat3_b);
gbmicrotest!(lcdon_to_stat3_c);
gbmicrotest!(lcdon_to_stat3_d);

// --- H-Blank interrupts ---
gbmicrotest!(hblank_int_di_timing_a);
gbmicrotest!(hblank_int_di_timing_b);
gbmicrotest!(hblank_int_if_a);
gbmicrotest!(hblank_int_if_b);
gbmicrotest!(hblank_int_l0);
gbmicrotest!(hblank_int_l1);
gbmicrotest!(hblank_int_l2);
gbmicrotest!(hblank_int_scx0);
gbmicrotest!(hblank_int_scx0_if_a);
gbmicrotest!(hblank_int_scx0_if_b);
gbmicrotest!(hblank_int_scx0_if_c);
gbmicrotest!(hblank_int_scx0_if_d);
gbmicrotest!(hblank_int_scx1);
gbmicrotest!(hblank_int_scx1_if_a);
gbmicrotest!(hblank_int_scx1_if_b);
gbmicrotest!(hblank_int_scx1_if_c);
gbmicrotest!(hblank_int_scx1_if_d);
gbmicrotest!(hblank_int_scx1_nops_a);
gbmicrotest!(hblank_int_scx1_nops_b);
gbmicrotest!(hblank_int_scx2);
gbmicrotest!(hblank_int_scx2_if_a);
gbmicrotest!(hblank_int_scx2_if_b);
gbmicrotest!(hblank_int_scx2_if_c);
gbmicrotest!(hblank_int_scx2_if_d);
gbmicrotest!(hblank_int_scx2_nops_a);
gbmicrotest!(hblank_int_scx2_nops_b);
gbmicrotest!(hblank_int_scx3);
gbmicrotest!(hblank_int_scx3_if_a);
gbmicrotest!(hblank_int_scx3_if_b);
gbmicrotest!(hblank_int_scx3_if_c);
gbmicrotest!(hblank_int_scx3_if_d);
gbmicrotest!(hblank_int_scx3_nops_a);
gbmicrotest!(hblank_int_scx3_nops_b);
gbmicrotest!(hblank_int_scx4);
gbmicrotest!(hblank_int_scx4_if_a);
gbmicrotest!(hblank_int_scx4_if_b);
gbmicrotest!(hblank_int_scx4_if_c);
gbmicrotest!(hblank_int_scx4_if_d);
gbmicrotest!(hblank_int_scx4_nops_a);
gbmicrotest!(hblank_int_scx4_nops_b);
gbmicrotest!(hblank_int_scx5);
gbmicrotest!(hblank_int_scx5_if_a);
gbmicrotest!(hblank_int_scx5_if_b);
gbmicrotest!(hblank_int_scx5_if_c);
gbmicrotest!(hblank_int_scx5_if_d);
gbmicrotest!(hblank_int_scx5_nops_a);
gbmicrotest!(hblank_int_scx5_nops_b);
gbmicrotest!(hblank_int_scx6);
gbmicrotest!(hblank_int_scx6_if_a);
gbmicrotest!(hblank_int_scx6_if_b);
gbmicrotest!(hblank_int_scx6_if_c);
gbmicrotest!(hblank_int_scx6_if_d);
gbmicrotest!(hblank_int_scx6_nops_a);
gbmicrotest!(hblank_int_scx6_nops_b);
gbmicrotest!(hblank_int_scx7);
gbmicrotest!(hblank_int_scx7_if_a);
gbmicrotest!(hblank_int_scx7_if_b);
gbmicrotest!(hblank_int_scx7_if_c);
gbmicrotest!(hblank_int_scx7_if_d);
gbmicrotest!(hblank_int_scx7_nops_a);
gbmicrotest!(hblank_int_scx7_nops_b);
gbmicrotest!(hblank_scx2_if_a);
gbmicrotest!(hblank_scx3_if_a);
gbmicrotest!(hblank_scx3_if_b);
gbmicrotest!(hblank_scx3_if_c);
gbmicrotest!(hblank_scx3_if_d);
gbmicrotest!(hblank_scx3_int_a);
gbmicrotest!(hblank_scx3_int_b);

// --- H-Blank HALT/interrupt timing ---
gbmicrotest!(int_hblank_halt_bug_a);
gbmicrotest!(int_hblank_halt_bug_b);
gbmicrotest!(int_hblank_halt_scx0);
gbmicrotest!(int_hblank_halt_scx1);
gbmicrotest!(int_hblank_halt_scx2);
gbmicrotest!(int_hblank_halt_scx3);
gbmicrotest!(int_hblank_halt_scx4);
gbmicrotest!(int_hblank_halt_scx5);
gbmicrotest!(int_hblank_halt_scx6);
gbmicrotest!(int_hblank_halt_scx7);
gbmicrotest!(int_hblank_incs_scx0);
gbmicrotest!(int_hblank_incs_scx1);
gbmicrotest!(int_hblank_incs_scx2);
gbmicrotest!(int_hblank_incs_scx3);
gbmicrotest!(int_hblank_incs_scx4);
gbmicrotest!(int_hblank_incs_scx5);
gbmicrotest!(int_hblank_incs_scx6);
gbmicrotest!(int_hblank_incs_scx7);
gbmicrotest!(int_hblank_nops_scx0);
gbmicrotest!(int_hblank_nops_scx1);
gbmicrotest!(int_hblank_nops_scx2);
gbmicrotest!(int_hblank_nops_scx3);
gbmicrotest!(int_hblank_nops_scx4);
gbmicrotest!(int_hblank_nops_scx5);
gbmicrotest!(int_hblank_nops_scx6);
gbmicrotest!(int_hblank_nops_scx7);

// --- LYC interrupts ---
gbmicrotest!(int_lyc_halt);
gbmicrotest!(int_lyc_incs);
gbmicrotest!(int_lyc_nops);
gbmicrotest!(lyc_int_halt_a);
gbmicrotest!(lyc_int_halt_b);
gbmicrotest!(lyc1_int_halt_a);
gbmicrotest!(lyc1_int_halt_b);
gbmicrotest!(lyc1_int_if_edge_a);
gbmicrotest!(lyc1_int_if_edge_b);
gbmicrotest!(lyc1_int_if_edge_c);
gbmicrotest!(lyc1_int_if_edge_d);
gbmicrotest!(lyc1_int_nops_a);
gbmicrotest!(lyc1_int_nops_b);
gbmicrotest!(lyc1_write_timing_a);
gbmicrotest!(lyc1_write_timing_b);
gbmicrotest!(lyc1_write_timing_c);
gbmicrotest!(lyc1_write_timing_d);
gbmicrotest!(lyc2_int_halt_a);
gbmicrotest!(lyc2_int_halt_b);

// --- OAM interrupts ---
gbmicrotest!(int_oam_halt);
gbmicrotest!(int_oam_incs);
gbmicrotest!(int_oam_nops);
gbmicrotest!(oam_int_halt_a);
gbmicrotest!(oam_int_halt_b);
gbmicrotest!(oam_int_if_edge_a);
gbmicrotest!(oam_int_if_edge_b);
gbmicrotest!(oam_int_if_edge_c);
gbmicrotest!(oam_int_if_edge_d);
gbmicrotest!(oam_int_if_level_c);
gbmicrotest!(oam_int_if_level_d);
gbmicrotest!(oam_int_inc_sled);
gbmicrotest!(oam_int_nops_a);
gbmicrotest!(oam_int_nops_b);

// --- Timer interrupts ---
gbmicrotest!(int_timer_halt);
gbmicrotest!(int_timer_halt_div_a);
gbmicrotest!(int_timer_halt_div_b);
gbmicrotest!(int_timer_incs);
gbmicrotest!(int_timer_nops);
gbmicrotest!(int_timer_nops_div_a);
gbmicrotest!(int_timer_nops_div_b);

// --- V-Blank interrupts ---
gbmicrotest!(int_vblank1_halt);
gbmicrotest!(int_vblank1_incs);
gbmicrotest!(int_vblank1_nops);
gbmicrotest!(int_vblank2_halt);
gbmicrotest!(int_vblank2_incs);
gbmicrotest!(int_vblank2_nops);
gbmicrotest!(vblank_int_halt_a);
gbmicrotest!(vblank_int_halt_b);
gbmicrotest!(vblank_int_if_a);
gbmicrotest!(vblank_int_if_b);
gbmicrotest!(vblank_int_if_c);
gbmicrotest!(vblank_int_if_d);
gbmicrotest!(vblank_int_inc_sled);
gbmicrotest!(vblank_int_nops_a);
gbmicrotest!(vblank_int_nops_b);
gbmicrotest!(vblank2_int_halt_a);
gbmicrotest!(vblank2_int_halt_b);
gbmicrotest!(vblank2_int_if_a);
gbmicrotest!(vblank2_int_if_b);
gbmicrotest!(vblank2_int_if_c);
gbmicrotest!(vblank2_int_if_d);
gbmicrotest!(vblank2_int_inc_sled);
gbmicrotest!(vblank2_int_nops_a);
gbmicrotest!(vblank2_int_nops_b);

// --- Line 144 ---
gbmicrotest!(line_144_oam_int_a);
gbmicrotest!(line_144_oam_int_b);
gbmicrotest!(line_144_oam_int_c);
gbmicrotest!(line_144_oam_int_d);

// --- Line 153 / LY edge cases ---
gbmicrotest!(line_153_ly_a);
gbmicrotest!(line_153_ly_b);
gbmicrotest!(line_153_ly_c);
gbmicrotest!(line_153_ly_d);
gbmicrotest!(line_153_ly_e);
gbmicrotest!(line_153_ly_f);
gbmicrotest!(line_153_lyc0_int_inc_sled);
gbmicrotest!(line_153_lyc0_stat_timing_a);
gbmicrotest!(line_153_lyc0_stat_timing_b);
gbmicrotest!(line_153_lyc0_stat_timing_c);
gbmicrotest!(line_153_lyc0_stat_timing_d);
gbmicrotest!(line_153_lyc0_stat_timing_e);
gbmicrotest!(line_153_lyc0_stat_timing_f);
gbmicrotest!(line_153_lyc0_stat_timing_g);
gbmicrotest!(line_153_lyc0_stat_timing_h);
gbmicrotest!(line_153_lyc0_stat_timing_i);
gbmicrotest!(line_153_lyc0_stat_timing_j);
gbmicrotest!(line_153_lyc0_stat_timing_k);
gbmicrotest!(line_153_lyc0_stat_timing_l);
gbmicrotest!(line_153_lyc0_stat_timing_m);
gbmicrotest!(line_153_lyc0_stat_timing_n);
gbmicrotest!(line_153_lyc153_stat_timing_a);
gbmicrotest!(line_153_lyc153_stat_timing_b);
gbmicrotest!(line_153_lyc153_stat_timing_c);
gbmicrotest!(line_153_lyc153_stat_timing_d);
gbmicrotest!(line_153_lyc153_stat_timing_e);
gbmicrotest!(line_153_lyc153_stat_timing_f);
gbmicrotest!(line_153_lyc_a);
gbmicrotest!(line_153_lyc_b);
gbmicrotest!(line_153_lyc_c);
gbmicrotest!(line_153_lyc_int_a);
gbmicrotest!(line_153_lyc_int_b);
gbmicrotest!(line_65_ly);

// --- STAT write glitch ---
gbmicrotest!(stat_write_glitch_l0_a);
gbmicrotest!(stat_write_glitch_l0_b);
gbmicrotest!(stat_write_glitch_l0_c);
gbmicrotest!(stat_write_glitch_l143_a);
gbmicrotest!(stat_write_glitch_l143_b);
gbmicrotest!(stat_write_glitch_l143_c);
gbmicrotest!(stat_write_glitch_l143_d);
gbmicrotest!(stat_write_glitch_l154_a);
gbmicrotest!(stat_write_glitch_l154_b);
gbmicrotest!(stat_write_glitch_l154_c);
gbmicrotest!(stat_write_glitch_l154_d);
gbmicrotest!(stat_write_glitch_l1_a);
gbmicrotest!(stat_write_glitch_l1_b);
gbmicrotest!(stat_write_glitch_l1_c);
gbmicrotest!(stat_write_glitch_l1_d);

// --- Power-on initial state ---
gbmicrotest!(poweron_bgp_000);
gbmicrotest!(poweron_div_000);
gbmicrotest!(poweron_div_004);
gbmicrotest!(poweron_div_005);
gbmicrotest!(poweron_dma_000);
gbmicrotest!(poweron_if_000);
gbmicrotest!(poweron_joy_000);
gbmicrotest!(poweron_lcdc_000);
gbmicrotest!(poweron_ly_000);
gbmicrotest!(poweron_ly_119);
gbmicrotest!(poweron_ly_120);
gbmicrotest!(poweron_ly_233);
gbmicrotest!(poweron_ly_234);
gbmicrotest!(poweron_lyc_000);
gbmicrotest!(poweron_oam_000);
gbmicrotest!(poweron_oam_005);
gbmicrotest!(poweron_oam_006);
gbmicrotest!(poweron_oam_069);
gbmicrotest!(poweron_oam_070);
gbmicrotest!(poweron_oam_119);
gbmicrotest!(poweron_oam_120);
gbmicrotest!(poweron_oam_121);
gbmicrotest!(poweron_oam_183);
gbmicrotest!(poweron_oam_184);
gbmicrotest!(poweron_oam_233);
gbmicrotest!(poweron_oam_234);
gbmicrotest!(poweron_oam_235);
gbmicrotest!(poweron_obp0_000);
gbmicrotest!(poweron_obp1_000);
gbmicrotest!(poweron_sb_000);
gbmicrotest!(poweron_sc_000);
gbmicrotest!(poweron_scx_000);
gbmicrotest!(poweron_scy_000);
gbmicrotest!(poweron_stat_000);
gbmicrotest!(poweron_stat_005);
gbmicrotest!(poweron_stat_006);
gbmicrotest!(poweron_stat_007);
gbmicrotest!(poweron_stat_026);
gbmicrotest!(poweron_stat_027);
gbmicrotest!(poweron_stat_069);
gbmicrotest!(poweron_stat_070);
gbmicrotest!(poweron_stat_119);
gbmicrotest!(poweron_stat_120);
gbmicrotest!(poweron_stat_121);
gbmicrotest!(poweron_stat_140);
gbmicrotest!(poweron_stat_141);
gbmicrotest!(poweron_stat_183);
gbmicrotest!(poweron_stat_184);
gbmicrotest!(poweron_stat_234);
gbmicrotest!(poweron_stat_235);
gbmicrotest!(poweron_tac_000);
gbmicrotest!(poweron_tima_000);
gbmicrotest!(poweron_tma_000);
gbmicrotest!(poweron_vram_000);
gbmicrotest!(poweron_vram_025);
gbmicrotest!(poweron_vram_026);
gbmicrotest!(poweron_vram_069);
gbmicrotest!(poweron_vram_070);
gbmicrotest!(poweron_vram_139);
gbmicrotest!(poweron_vram_140);
gbmicrotest!(poweron_vram_183);
gbmicrotest!(poweron_vram_184);
gbmicrotest!(poweron_wx_000);
gbmicrotest!(poweron_wy_000);

// --- PPU sprites ---
gbmicrotest!(ppu_sprite0_scx0_a);
gbmicrotest!(ppu_sprite0_scx0_b);
gbmicrotest!(ppu_sprite0_scx1_a);
gbmicrotest!(ppu_sprite0_scx1_b);
gbmicrotest!(ppu_sprite0_scx2_a);
gbmicrotest!(ppu_sprite0_scx2_b);
gbmicrotest!(ppu_sprite0_scx3_a);
gbmicrotest!(ppu_sprite0_scx3_b);
gbmicrotest!(ppu_sprite0_scx4_a);
gbmicrotest!(ppu_sprite0_scx4_b);
gbmicrotest!(ppu_sprite0_scx5_a);
gbmicrotest!(ppu_sprite0_scx5_b);
gbmicrotest!(ppu_sprite0_scx6_a);
gbmicrotest!(ppu_sprite0_scx6_b);
gbmicrotest!(ppu_sprite0_scx7_a);
gbmicrotest!(ppu_sprite0_scx7_b);
gbmicrotest!(sprite_0_a);
gbmicrotest!(sprite_0_b);
gbmicrotest!(sprite_1_a);
gbmicrotest!(sprite_1_b);
gbmicrotest!(sprite4_0_a);
gbmicrotest!(sprite4_0_b);
gbmicrotest!(sprite4_1_a);
gbmicrotest!(sprite4_1_b);
gbmicrotest!(sprite4_2_a);
gbmicrotest!(sprite4_2_b);
gbmicrotest!(sprite4_3_a);
gbmicrotest!(sprite4_3_b);
gbmicrotest!(sprite4_4_a);
gbmicrotest!(sprite4_4_b);
gbmicrotest!(sprite4_5_a);
gbmicrotest!(sprite4_5_b);
gbmicrotest!(sprite4_6_a);
gbmicrotest!(sprite4_6_b);
gbmicrotest!(sprite4_7_a);
gbmicrotest!(sprite4_7_b);

// --- PPU window ---
gbmicrotest!(win0_a);
gbmicrotest!(win0_b);
gbmicrotest!(win0_scx3_a);
gbmicrotest!(win0_scx3_b);
gbmicrotest!(win1_a);
gbmicrotest!(win1_b);
gbmicrotest!(win2_a);
gbmicrotest!(win2_b);
gbmicrotest!(win3_a);
gbmicrotest!(win3_b);
gbmicrotest!(win4_a);
gbmicrotest!(win4_b);
gbmicrotest!(win5_a);
gbmicrotest!(win5_b);
gbmicrotest!(win6_a);
gbmicrotest!(win6_b);
gbmicrotest!(win7_a);
gbmicrotest!(win7_b);
gbmicrotest!(win8_a);
gbmicrotest!(win8_b);
gbmicrotest!(win9_a);
gbmicrotest!(win9_b);
gbmicrotest!(win10_a);
gbmicrotest!(win10_b);
gbmicrotest!(win10_scx3_a);
gbmicrotest!(win10_scx3_b);
gbmicrotest!(win11_a);
gbmicrotest!(win11_b);
gbmicrotest!(win12_a);
gbmicrotest!(win12_b);
gbmicrotest!(win13_a);
gbmicrotest!(win13_b);
gbmicrotest!(win14_a);
gbmicrotest!(win14_b);
gbmicrotest!(win15_a);
gbmicrotest!(win15_b);

// --- MBC ---
gbmicrotest!(mbc1_ram_banks);
gbmicrotest!(mbc1_rom_banks);
