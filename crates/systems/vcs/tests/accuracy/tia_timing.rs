use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn hmove_apply_ntsc() {
    common::run_self_test("tia-timing/hmove-apply_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_apply_pal() {
    common::run_self_test("tia-timing/hmove-apply_pal.a26", TvStandard::Pal);
}

#[test]
fn hmove_apply_secam() {
    common::run_self_test("tia-timing/hmove-apply_secam.a26", TvStandard::Secam);
}

#[test]
fn hmove_comb_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-comb_ntsc.a26",
        "tia-timing/hmove-comb_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_comb_pal() {
    common::run_screenshot(
        "tia-timing/hmove-comb_pal.a26",
        "tia-timing/hmove-comb_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_comb_secam() {
    common::run_screenshot(
        "tia-timing/hmove-comb_secam.a26",
        "tia-timing/hmove-comb_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_late_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-late_ntsc.a26",
        "tia-timing/hmove-late_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_late_pal() {
    common::run_screenshot(
        "tia-timing/hmove-late_pal.a26",
        "tia-timing/hmove-late_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_late_secam() {
    common::run_screenshot(
        "tia-timing/hmove-late_secam.a26",
        "tia-timing/hmove-late_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_line_aligned_ntsc() {
    common::run_self_test("tia-timing/hmove-line-aligned_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_line_aligned_pal() {
    common::run_self_test("tia-timing/hmove-line-aligned_pal.a26", TvStandard::Pal);
}

#[test]
fn hmove_line_aligned_secam() {
    common::run_self_test("tia-timing/hmove-line-aligned_secam.a26", TvStandard::Secam);
}

#[test]
fn hmove_line_end_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-line-end_ntsc.a26",
        "tia-timing/hmove-line-end_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_line_end_pal() {
    common::run_screenshot(
        "tia-timing/hmove-line-end_pal.a26",
        "tia-timing/hmove-line-end_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_line_end_secam() {
    common::run_screenshot(
        "tia-timing/hmove-line-end_secam.a26",
        "tia-timing/hmove-line-end_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_live_seam_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-live-seam_ntsc.a26",
        "tia-timing/hmove-live-seam_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_live_seam_pal() {
    common::run_screenshot(
        "tia-timing/hmove-live-seam_pal.a26",
        "tia-timing/hmove-live-seam_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_live_seam_secam() {
    common::run_screenshot(
        "tia-timing/hmove-live-seam_secam.a26",
        "tia-timing/hmove-live-seam_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_live_reach_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-live-reach_ntsc.a26",
        "tia-timing/hmove-live-reach_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_live_reach_pal() {
    common::run_screenshot(
        "tia-timing/hmove-live-reach_pal.a26",
        "tia-timing/hmove-live-reach_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_live_reach_secam() {
    common::run_screenshot(
        "tia-timing/hmove-live-reach_secam.a26",
        "tia-timing/hmove-live-reach_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_strobe_line_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-strobe-line_ntsc.a26",
        "tia-timing/hmove-strobe-line_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_strobe_line_pal() {
    common::run_screenshot(
        "tia-timing/hmove-strobe-line_pal.a26",
        "tia-timing/hmove-strobe-line_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_strobe_line_secam() {
    common::run_screenshot(
        "tia-timing/hmove-strobe-line_secam.a26",
        "tia-timing/hmove-strobe-line_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_reset_merge_ntsc() {
    common::run_self_test("tia-timing/hmove-reset-merge_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_reset_merge_pal() {
    common::run_self_test("tia-timing/hmove-reset-merge_pal.a26", TvStandard::Pal);
}

#[test]
fn hmove_reset_merge_secam() {
    common::run_self_test("tia-timing/hmove-reset-merge_secam.a26", TvStandard::Secam);
}

#[test]
fn hmove_rewrite_race_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-rewrite-race_ntsc.a26",
        "tia-timing/hmove-rewrite-race_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_rewrite_race_pal() {
    common::run_screenshot(
        "tia-timing/hmove-rewrite-race_pal.a26",
        "tia-timing/hmove-rewrite-race_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_rewrite_race_secam() {
    common::run_screenshot(
        "tia-timing/hmove-rewrite-race_secam.a26",
        "tia-timing/hmove-rewrite-race_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_grid_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-grid_ntsc.a26",
        "tia-timing/hmove-stuck-grid_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_grid_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-grid_pal.a26",
        "tia-timing/hmove-stuck-grid_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_grid_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-grid_secam.a26",
        "tia-timing/hmove-stuck-grid_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_latch_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-latch_ntsc.a26",
        "tia-timing/hmove-stuck-latch_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_latch_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-latch_pal.a26",
        "tia-timing/hmove-stuck-latch_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_latch_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-latch_secam.a26",
        "tia-timing/hmove-stuck-latch_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_player_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-player_ntsc.a26",
        "tia-timing/hmove-stuck-player_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_player_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-player_pal.a26",
        "tia-timing/hmove-stuck-player_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_player_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-player_secam.a26",
        "tia-timing/hmove-stuck-player_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_straddle_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-straddle_ntsc.a26",
        "tia-timing/hmove-stuck-straddle_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_straddle_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-straddle_pal.a26",
        "tia-timing/hmove-stuck-straddle_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_straddle_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-straddle_secam.a26",
        "tia-timing/hmove-stuck-straddle_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_release_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-release_ntsc.a26",
        "tia-timing/hmove-stuck-release_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_release_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-release_pal.a26",
        "tia-timing/hmove-stuck-release_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_release_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-release_secam.a26",
        "tia-timing/hmove-stuck-release_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_widths_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-widths_ntsc.a26",
        "tia-timing/hmove-stuck-widths_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_widths_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-widths_pal.a26",
        "tia-timing/hmove-stuck-widths_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_widths_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-widths_secam.a26",
        "tia-timing/hmove-stuck-widths_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_stuck_stretch_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-stretch_ntsc.a26",
        "tia-timing/hmove-stuck-stretch_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_stuck_stretch_pal() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-stretch_pal.a26",
        "tia-timing/hmove-stuck-stretch_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_stuck_stretch_secam() {
    common::run_screenshot(
        "tia-timing/hmove-stuck-stretch_secam.a26",
        "tia-timing/hmove-stuck-stretch_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_values_ntsc() {
    common::run_screenshot(
        "tia-timing/hmove-values_ntsc.a26",
        "tia-timing/hmove-values_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn hmove_values_pal() {
    common::run_screenshot(
        "tia-timing/hmove-values_pal.a26",
        "tia-timing/hmove-values_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn hmove_values_secam() {
    common::run_screenshot(
        "tia-timing/hmove-values_secam.a26",
        "tia-timing/hmove-values_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn hmove_walk_ntsc() {
    common::run_self_test("tia-timing/hmove-walk_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_walk_pal() {
    common::run_self_test("tia-timing/hmove-walk_pal.a26", TvStandard::Pal);
}

#[test]
fn hmove_walk_secam() {
    common::run_self_test("tia-timing/hmove-walk_secam.a26", TvStandard::Secam);
}

#[test]
fn midline_color_ntsc() {
    common::run_screenshot(
        "tia-timing/midline-color_ntsc.a26",
        "tia-timing/midline-color_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn midline_color_pal() {
    common::run_screenshot(
        "tia-timing/midline-color_pal.a26",
        "tia-timing/midline-color_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn midline_color_secam() {
    common::run_screenshot(
        "tia-timing/midline-color_secam.a26",
        "tia-timing/midline-color_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn midline_grp_ntsc() {
    common::run_screenshot(
        "tia-timing/midline-grp_ntsc.a26",
        "tia-timing/midline-grp_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn midline_grp_pal() {
    common::run_screenshot(
        "tia-timing/midline-grp_pal.a26",
        "tia-timing/midline-grp_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn midline_grp_secam() {
    common::run_screenshot(
        "tia-timing/midline-grp_secam.a26",
        "tia-timing/midline-grp_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn midline_pf_ntsc() {
    common::run_screenshot(
        "tia-timing/midline-pf_ntsc.a26",
        "tia-timing/midline-pf_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn midline_pf_pal() {
    common::run_screenshot(
        "tia-timing/midline-pf_pal.a26",
        "tia-timing/midline-pf_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn midline_pf_secam() {
    common::run_screenshot(
        "tia-timing/midline-pf_secam.a26",
        "tia-timing/midline-pf_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn midline_resp_ntsc() {
    common::run_screenshot(
        "tia-timing/midline-resp_ntsc.a26",
        "tia-timing/midline-resp_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn midline_resp_pal() {
    common::run_screenshot(
        "tia-timing/midline-resp_pal.a26",
        "tia-timing/midline-resp_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn midline_resp_secam() {
    common::run_screenshot(
        "tia-timing/midline-resp_secam.a26",
        "tia-timing/midline-resp_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn midline_vblank_ntsc() {
    common::run_screenshot(
        "tia-timing/midline-vblank_ntsc.a26",
        "tia-timing/midline-vblank_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn midline_vblank_pal() {
    common::run_screenshot(
        "tia-timing/midline-vblank_pal.a26",
        "tia-timing/midline-vblank_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn midline_vblank_secam() {
    common::run_screenshot(
        "tia-timing/midline-vblank_secam.a26",
        "tia-timing/midline-vblank_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn nusiz_draw_ntsc() {
    common::run_screenshot(
        "tia-timing/nusiz-draw_ntsc.a26",
        "tia-timing/nusiz-draw_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn nusiz_draw_pal() {
    common::run_screenshot(
        "tia-timing/nusiz-draw_pal.a26",
        "tia-timing/nusiz-draw_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn nusiz_draw_secam() {
    common::run_screenshot(
        "tia-timing/nusiz-draw_secam.a26",
        "tia-timing/nusiz-draw_secam.png",
        TvStandard::Secam,
    );
}

#[test]
fn reset_same_line_ntsc() {
    common::run_self_test("tia-timing/reset-same-line_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn reset_same_line_pal() {
    common::run_self_test("tia-timing/reset-same-line_pal.a26", TvStandard::Pal);
}

#[test]
fn reset_same_line_secam() {
    common::run_self_test("tia-timing/reset-same-line_secam.a26", TvStandard::Secam);
}

#[test]
fn resp_restrobe_ntsc() {
    common::run_self_test("tia-timing/resp-restrobe_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn resp_restrobe_pal() {
    common::run_self_test("tia-timing/resp-restrobe_pal.a26", TvStandard::Pal);
}

#[test]
fn resp_restrobe_secam() {
    common::run_self_test("tia-timing/resp-restrobe_secam.a26", TvStandard::Secam);
}

#[test]
fn rsync_ntsc() {
    common::run_self_test("tia-timing/rsync_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn rsync_pal() {
    common::run_self_test("tia-timing/rsync_pal.a26", TvStandard::Pal);
}

#[test]
fn rsync_secam() {
    common::run_self_test("tia-timing/rsync_secam.a26", TvStandard::Secam);
}

#[test]
fn wsync_line_end_ntsc() {
    common::run_self_test("tia-timing/wsync-line-end_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn wsync_line_end_pal() {
    common::run_self_test("tia-timing/wsync-line-end_pal.a26", TvStandard::Pal);
}

#[test]
fn wsync_line_end_secam() {
    common::run_self_test("tia-timing/wsync-line-end_secam.a26", TvStandard::Secam);
}

#[test]
fn wsync_ntsc() {
    common::run_self_test("tia-timing/wsync_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn wsync_pal() {
    common::run_self_test("tia-timing/wsync_pal.a26", TvStandard::Pal);
}

#[test]
fn wsync_secam() {
    common::run_self_test("tia-timing/wsync_secam.a26", TvStandard::Secam);
}
