use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn sanity_ntsc() {
    common::run_self_test("harness/sanity_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn sanity_pal() {
    common::run_self_test("harness/sanity_pal.a26", TvStandard::Pal);
}

#[test]
fn sanity_secam() {
    common::run_self_test("harness/sanity_secam.a26", TvStandard::Secam);
}

// The capture-rig target: not a behaviour test upstream, but every oracle
// renders it identically, so the blessed render doubles as a consensus
// screenshot reference.
#[test]
fn calibration_ntsc() {
    common::run_screenshot(
        "harness/calibration_ntsc.a26",
        "harness/calibration_ntsc.png",
        TvStandard::Ntsc,
    );
}

#[test]
fn calibration_pal() {
    common::run_screenshot(
        "harness/calibration_pal.a26",
        "harness/calibration_pal.png",
        TvStandard::Pal,
    );
}

#[test]
fn calibration_secam() {
    common::run_screenshot(
        "harness/calibration_secam.a26",
        "harness/calibration_secam.png",
        TvStandard::Secam,
    );
}
