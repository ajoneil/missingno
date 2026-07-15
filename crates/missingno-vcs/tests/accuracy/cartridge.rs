//! Cartridge board tests. Each ROM states the board it is wired for — the
//! suite never infers one from the image, since the board is the subject.

use crate::common::run_self_test_on;
use missingno_vcs::{CartType, TvStandard};

#[test]
fn mirror_2k_ntsc() {
    run_self_test_on(
        "cartridge/mirror-2k_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Plain2K,
    );
}

#[test]
fn mirror_2k_pal() {
    run_self_test_on(
        "cartridge/mirror-2k_pal.a26",
        TvStandard::Pal,
        CartType::Plain2K,
    );
}

#[test]
fn mirror_2k_secam() {
    run_self_test_on(
        "cartridge/mirror-2k_secam.a26",
        TvStandard::Secam,
        CartType::Plain2K,
    );
}

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

#[test]
fn bank_fa_ntsc() {
    run_self_test_on("cartridge/bank-fa_ntsc.a26", TvStandard::Ntsc, CartType::Fa);
}

#[test]
fn bank_fa_pal() {
    run_self_test_on("cartridge/bank-fa_pal.a26", TvStandard::Pal, CartType::Fa);
}

#[test]
fn bank_fa_secam() {
    run_self_test_on(
        "cartridge/bank-fa_secam.a26",
        TvStandard::Secam,
        CartType::Fa,
    );
}

#[test]
fn bank_e0_ntsc() {
    run_self_test_on("cartridge/bank-e0_ntsc.a26", TvStandard::Ntsc, CartType::E0);
}

#[test]
fn bank_e0_pal() {
    run_self_test_on("cartridge/bank-e0_pal.a26", TvStandard::Pal, CartType::E0);
}

#[test]
fn bank_e0_secam() {
    run_self_test_on(
        "cartridge/bank-e0_secam.a26",
        TvStandard::Secam,
        CartType::E0,
    );
}

#[test]
fn bank_e7_ntsc() {
    run_self_test_on("cartridge/bank-e7_ntsc.a26", TvStandard::Ntsc, CartType::E7);
}

#[test]
fn bank_e7_pal() {
    run_self_test_on("cartridge/bank-e7_pal.a26", TvStandard::Pal, CartType::E7);
}

#[test]
fn bank_e7_secam() {
    run_self_test_on(
        "cartridge/bank-e7_secam.a26",
        TvStandard::Secam,
        CartType::E7,
    );
}

#[test]
fn bank_fe_ntsc() {
    run_self_test_on("cartridge/bank-fe_ntsc.a26", TvStandard::Ntsc, CartType::Fe);
}

#[test]
fn bank_fe_pal() {
    run_self_test_on("cartridge/bank-fe_pal.a26", TvStandard::Pal, CartType::Fe);
}

#[test]
fn bank_fe_secam() {
    run_self_test_on(
        "cartridge/bank-fe_secam.a26",
        TvStandard::Secam,
        CartType::Fe,
    );
}

#[test]
fn bank_3f_ntsc() {
    run_self_test_on(
        "cartridge/bank-3f_ntsc.a26",
        TvStandard::Ntsc,
        CartType::ThreeF,
    );
}

#[test]
fn bank_3f_pal() {
    run_self_test_on(
        "cartridge/bank-3f_pal.a26",
        TvStandard::Pal,
        CartType::ThreeF,
    );
}

#[test]
fn bank_3f_secam() {
    run_self_test_on(
        "cartridge/bank-3f_secam.a26",
        TvStandard::Secam,
        CartType::ThreeF,
    );
}

#[test]
fn bank_ua_ntsc() {
    run_self_test_on("cartridge/bank-ua_ntsc.a26", TvStandard::Ntsc, CartType::Ua);
}

#[test]
fn bank_ua_pal() {
    run_self_test_on("cartridge/bank-ua_pal.a26", TvStandard::Pal, CartType::Ua);
}

#[test]
fn bank_ua_secam() {
    run_self_test_on(
        "cartridge/bank-ua_secam.a26",
        TvStandard::Secam,
        CartType::Ua,
    );
}

#[test]
fn ram_cv_ntsc() {
    run_self_test_on("cartridge/ram-cv_ntsc.a26", TvStandard::Ntsc, CartType::Cv);
}

#[test]
fn ram_cv_pal() {
    run_self_test_on("cartridge/ram-cv_pal.a26", TvStandard::Pal, CartType::Cv);
}

#[test]
fn ram_cv_secam() {
    run_self_test_on(
        "cartridge/ram-cv_secam.a26",
        TvStandard::Secam,
        CartType::Cv,
    );
}

#[test]
fn bank_hotspot_window_ntsc() {
    run_self_test_on(
        "cartridge/bank-hotspot-window_ntsc.a26",
        TvStandard::Ntsc,
        CartType::F8,
    );
}

#[test]
fn bank_hotspot_window_pal() {
    run_self_test_on(
        "cartridge/bank-hotspot-window_pal.a26",
        TvStandard::Pal,
        CartType::F8,
    );
}

#[test]
fn bank_hotspot_window_secam() {
    run_self_test_on(
        "cartridge/bank-hotspot-window_secam.a26",
        TvStandard::Secam,
        CartType::F8,
    );
}

#[test]
fn ram_superchip_ntsc() {
    run_self_test_on(
        "cartridge/ram-superchip_ntsc.a26",
        TvStandard::Ntsc,
        CartType::F8Sc,
    );
}

#[test]
fn ram_superchip_pal() {
    run_self_test_on(
        "cartridge/ram-superchip_pal.a26",
        TvStandard::Pal,
        CartType::F8Sc,
    );
}

#[test]
fn ram_superchip_secam() {
    run_self_test_on(
        "cartridge/ram-superchip_secam.a26",
        TvStandard::Secam,
        CartType::F8Sc,
    );
}
