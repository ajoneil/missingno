use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn all_pairs_ntsc() {
    common::run_self_test("collision/all-pairs_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn all_pairs_pal() {
    common::run_self_test("collision/all-pairs_pal.a26", TvStandard::Pal);
}

#[test]
fn hmove_edge_ntsc() {
    common::run_self_test("collision/hmove-edge_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn hmove_edge_pal() {
    common::run_self_test("collision/hmove-edge_pal.a26", TvStandard::Pal);
}

#[test]
fn latches_ntsc() {
    common::run_self_test("collision/latches_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn latches_pal() {
    common::run_self_test("collision/latches_pal.a26", TvStandard::Pal);
}

#[test]
fn onset_sweep_ntsc() {
    common::run_self_test("collision/onset-sweep_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn onset_sweep_pal() {
    common::run_self_test("collision/onset-sweep_pal.a26", TvStandard::Pal);
}

#[test]
fn per_pixel_ntsc() {
    common::run_self_test("collision/per-pixel_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn per_pixel_pal() {
    common::run_self_test("collision/per-pixel_pal.a26", TvStandard::Pal);
}
