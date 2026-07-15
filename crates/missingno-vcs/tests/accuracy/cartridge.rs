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
