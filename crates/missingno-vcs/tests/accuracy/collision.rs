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
fn all_pairs_secam() {
    common::run_self_test("collision/all-pairs_secam.a26", TvStandard::Secam);
}

#[test]
fn blank_gating_ntsc() {
    common::run_self_test("collision/blank-gating_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn blank_gating_pal() {
    common::run_self_test("collision/blank-gating_pal.a26", TvStandard::Pal);
}

#[test]
fn blank_gating_secam() {
    common::run_self_test("collision/blank-gating_secam.a26", TvStandard::Secam);
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
fn hmove_edge_secam() {
    common::run_self_test("collision/hmove-edge_secam.a26", TvStandard::Secam);
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
fn latches_secam() {
    common::run_self_test("collision/latches_secam.a26", TvStandard::Secam);
}

#[test]
fn latency_swallow_ntsc() {
    common::run_self_test("collision/latency-swallow_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn latency_swallow_pal() {
    common::run_self_test("collision/latency-swallow_pal.a26", TvStandard::Pal);
}

#[test]
fn latency_swallow_secam() {
    common::run_self_test("collision/latency-swallow_secam.a26", TvStandard::Secam);
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
fn onset_sweep_secam() {
    common::run_self_test("collision/onset-sweep_secam.a26", TvStandard::Secam);
}

#[test]
fn per_pixel_ntsc() {
    common::run_self_test("collision/per-pixel_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn per_pixel_pal() {
    common::run_self_test("collision/per-pixel_pal.a26", TvStandard::Pal);
}

#[test]
fn per_pixel_secam() {
    common::run_self_test("collision/per-pixel_secam.a26", TvStandard::Secam);
}

#[test]
fn reset_phase_ntsc() {
    common::run_self_test("collision/reset-phase_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn reset_phase_pal() {
    common::run_self_test("collision/reset-phase_pal.a26", TvStandard::Pal);
}

#[test]
fn reset_phase_secam() {
    common::run_self_test("collision/reset-phase_secam.a26", TvStandard::Secam);
}

#[test]
fn stuck_drift_ntsc() {
    common::run_self_test("collision/stuck-drift_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn stuck_drift_pal() {
    common::run_self_test("collision/stuck-drift_pal.a26", TvStandard::Pal);
}

#[test]
fn stuck_drift_secam() {
    common::run_self_test("collision/stuck-drift_secam.a26", TvStandard::Secam);
}
