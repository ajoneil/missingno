//! Cartridge board tests. Each ROM states the board it is wired for — the
//! suite never infers one from the image, since the board is the subject.

use crate::common::run_self_test_on;
use missingno_vcs::{CartType, TvStandard};

#[test]
fn bank_f4_ntsc() {
    run_self_test_on("cartridge/bank-f4_ntsc.a26", TvStandard::Ntsc, CartType::F4);
}

#[test]
fn bank_f4_pal() {
    run_self_test_on("cartridge/bank-f4_pal.a26", TvStandard::Pal, CartType::F4);
}

#[test]
fn bank_f4_secam() {
    run_self_test_on(
        "cartridge/bank-f4_secam.a26",
        TvStandard::Secam,
        CartType::F4,
    );
}

#[test]
fn bank_f6_ntsc() {
    run_self_test_on("cartridge/bank-f6_ntsc.a26", TvStandard::Ntsc, CartType::F6);
}

#[test]
fn bank_f6_pal() {
    run_self_test_on("cartridge/bank-f6_pal.a26", TvStandard::Pal, CartType::F6);
}

#[test]
fn bank_f6_secam() {
    run_self_test_on(
        "cartridge/bank-f6_secam.a26",
        TvStandard::Secam,
        CartType::F6,
    );
}

#[test]
fn bank_f8_ntsc() {
    run_self_test_on("cartridge/bank-f8_ntsc.a26", TvStandard::Ntsc, CartType::F8);
}

#[test]
fn bank_f8_pal() {
    run_self_test_on("cartridge/bank-f8_pal.a26", TvStandard::Pal, CartType::F8);
}

#[test]
fn bank_f8_secam() {
    run_self_test_on(
        "cartridge/bank-f8_secam.a26",
        TvStandard::Secam,
        CartType::F8,
    );
}
