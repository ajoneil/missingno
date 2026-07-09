use crate::common;

#[test]
fn sanity_ntsc() {
    common::run_self_test("harness/sanity_ntsc.a26");
}

#[test]
fn sanity_pal() {
    common::run_self_test("harness/sanity_pal.a26");
}
