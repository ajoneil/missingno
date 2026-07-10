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
fn hmove_line_aligned_ntsc() {
    common::run_self_test("tia-timing/hmove-line-aligned_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_line_aligned_pal() {
    common::run_self_test("tia-timing/hmove-line-aligned_pal.a26", TvStandard::Pal);
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
fn hmove_walk_ntsc() {
    common::run_self_test("tia-timing/hmove-walk_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_walk_pal() {
    common::run_self_test("tia-timing/hmove-walk_pal.a26", TvStandard::Pal);
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
fn reset_same_line_ntsc() {
    common::run_self_test("tia-timing/reset-same-line_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn reset_same_line_pal() {
    common::run_self_test("tia-timing/reset-same-line_pal.a26", TvStandard::Pal);
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
fn wsync_ntsc() {
    common::run_self_test("tia-timing/wsync_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn wsync_pal() {
    common::run_self_test("tia-timing/wsync_pal.a26", TvStandard::Pal);
}
