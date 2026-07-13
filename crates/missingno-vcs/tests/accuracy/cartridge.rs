use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn bank_f4_ntsc() {
    common::run_self_test("cartridge/bank-f4_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn bank_f4_pal() {
    common::run_self_test("cartridge/bank-f4_pal.a26", TvStandard::Pal);
}

#[test]
fn bank_f4_secam() {
    common::run_self_test("cartridge/bank-f4_secam.a26", TvStandard::Secam);
}

#[test]
fn bank_f6_ntsc() {
    common::run_self_test("cartridge/bank-f6_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn bank_f6_pal() {
    common::run_self_test("cartridge/bank-f6_pal.a26", TvStandard::Pal);
}

#[test]
fn bank_f6_secam() {
    common::run_self_test("cartridge/bank-f6_secam.a26", TvStandard::Secam);
}

#[test]
fn bank_f8_ntsc() {
    common::run_self_test("cartridge/bank-f8_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn bank_f8_pal() {
    common::run_self_test("cartridge/bank-f8_pal.a26", TvStandard::Pal);
}

#[test]
fn bank_f8_secam() {
    common::run_self_test("cartridge/bank-f8_secam.a26", TvStandard::Secam);
}
