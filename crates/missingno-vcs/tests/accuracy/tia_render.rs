use crate::common;

#[test]
fn colors_ntsc() {
    common::run_screenshot("tia-render/colors_ntsc.a26", "tia-render/colors_ntsc.png");
}

#[test]
fn colors_pal() {
    common::run_screenshot("tia-render/colors_pal.a26", "tia-render/colors_pal.png");
}

#[test]
fn draw_delay_ntsc() {
    common::run_screenshot(
        "tia-render/draw-delay_ntsc.a26",
        "tia-render/draw-delay_ntsc.png",
    );
}

#[test]
fn draw_delay_pal() {
    common::run_screenshot(
        "tia-render/draw-delay_pal.a26",
        "tia-render/draw-delay_pal.png",
    );
}

#[test]
fn missile_reset_lock_ntsc() {
    common::run_self_test("tia-render/missile-reset-lock_ntsc.a26");
}

#[test]
fn missile_reset_lock_pal() {
    common::run_self_test("tia-render/missile-reset-lock_pal.a26");
}

#[test]
fn missiles_ball_ntsc() {
    common::run_self_test("tia-render/missiles-ball_ntsc.a26");
}

#[test]
fn missiles_ball_pal() {
    common::run_self_test("tia-render/missiles-ball_pal.a26");
}

#[test]
fn nusiz_ntsc() {
    common::run_self_test("tia-render/nusiz_ntsc.a26");
}

#[test]
fn nusiz_pal() {
    common::run_self_test("tia-render/nusiz_pal.a26");
}

#[test]
fn object_priority_ntsc() {
    common::run_screenshot(
        "tia-render/object-priority_ntsc.a26",
        "tia-render/object-priority_ntsc.png",
    );
}

#[test]
fn object_priority_pal() {
    common::run_screenshot(
        "tia-render/object-priority_pal.a26",
        "tia-render/object-priority_pal.png",
    );
}

#[test]
fn pf_priority_ntsc() {
    common::run_screenshot(
        "tia-render/pf-priority_ntsc.a26",
        "tia-render/pf-priority_ntsc.png",
    );
}

#[test]
fn pf_priority_pal() {
    common::run_screenshot(
        "tia-render/pf-priority_pal.a26",
        "tia-render/pf-priority_pal.png",
    );
}

#[test]
fn player_reflect_ntsc() {
    common::run_self_test("tia-render/player-reflect_ntsc.a26");
}

#[test]
fn player_reflect_pal() {
    common::run_self_test("tia-render/player-reflect_pal.a26");
}

#[test]
fn players_ntsc() {
    common::run_self_test("tia-render/players_ntsc.a26");
}

#[test]
fn players_pal() {
    common::run_self_test("tia-render/players_pal.a26");
}

#[test]
fn playfield_ntsc() {
    common::run_self_test("tia-render/playfield_ntsc.a26");
}

#[test]
fn playfield_pal() {
    common::run_self_test("tia-render/playfield_pal.a26");
}

#[test]
fn playfield_reflect_ntsc() {
    common::run_self_test("tia-render/playfield-reflect_ntsc.a26");
}

#[test]
fn playfield_reflect_pal() {
    common::run_self_test("tia-render/playfield-reflect_pal.a26");
}

#[test]
fn positioning_ntsc() {
    common::run_self_test("tia-render/positioning_ntsc.a26");
}

#[test]
fn positioning_pal() {
    common::run_self_test("tia-render/positioning_pal.a26");
}

#[test]
fn score_mode_ntsc() {
    common::run_screenshot(
        "tia-render/score-mode_ntsc.a26",
        "tia-render/score-mode_ntsc.png",
    );
}

#[test]
fn score_mode_pal() {
    common::run_screenshot(
        "tia-render/score-mode_pal.a26",
        "tia-render/score-mode_pal.png",
    );
}

#[test]
fn vertical_delay_ntsc() {
    common::run_self_test("tia-render/vertical-delay_ntsc.a26");
}

#[test]
fn vertical_delay_pal() {
    common::run_self_test("tia-render/vertical-delay_pal.a26");
}
