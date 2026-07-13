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
