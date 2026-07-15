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

#[test]
fn dpc_fetch_ntsc() {
    run_self_test_on(
        "cartridge/dpc-fetch_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_fetch_pal() {
    run_self_test_on(
        "cartridge/dpc-fetch_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_fetch_secam() {
    run_self_test_on(
        "cartridge/dpc-fetch_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_flag_ntsc() {
    run_self_test_on(
        "cartridge/dpc-flag_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_flag_pal() {
    run_self_test_on("cartridge/dpc-flag_pal.a26", TvStandard::Pal, CartType::Dpc);
}

#[test]
fn dpc_flag_secam() {
    run_self_test_on(
        "cartridge/dpc-flag_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_bank_ntsc() {
    run_self_test_on(
        "cartridge/dpc-bank_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_bank_pal() {
    run_self_test_on("cartridge/dpc-bank_pal.a26", TvStandard::Pal, CartType::Dpc);
}

#[test]
fn dpc_bank_secam() {
    run_self_test_on(
        "cartridge/dpc-bank_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_swizzle_ntsc() {
    run_self_test_on(
        "cartridge/dpc-swizzle_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_swizzle_pal() {
    run_self_test_on(
        "cartridge/dpc-swizzle_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_swizzle_secam() {
    run_self_test_on(
        "cartridge/dpc-swizzle_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_shift_ntsc() {
    run_self_test_on(
        "cartridge/dpc-shift_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_shift_pal() {
    run_self_test_on(
        "cartridge/dpc-shift_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_shift_secam() {
    run_self_test_on(
        "cartridge/dpc-shift_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_window_reads_ntsc() {
    run_self_test_on(
        "cartridge/dpc-window-reads_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_window_reads_pal() {
    run_self_test_on(
        "cartridge/dpc-window-reads_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_window_reads_secam() {
    run_self_test_on(
        "cartridge/dpc-window-reads_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_flag_edges_ntsc() {
    run_self_test_on(
        "cartridge/dpc-flag-edges_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_flag_edges_pal() {
    run_self_test_on(
        "cartridge/dpc-flag-edges_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_flag_edges_secam() {
    run_self_test_on(
        "cartridge/dpc-flag-edges_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_rng_ntsc() {
    run_self_test_on(
        "cartridge/dpc-rng_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_rng_pal() {
    run_self_test_on("cartridge/dpc-rng_pal.a26", TvStandard::Pal, CartType::Dpc);
}

#[test]
fn dpc_rng_secam() {
    run_self_test_on(
        "cartridge/dpc-rng_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_music_ntsc() {
    run_self_test_on(
        "cartridge/dpc-music_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_music_pal() {
    run_self_test_on(
        "cartridge/dpc-music_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_music_secam() {
    run_self_test_on(
        "cartridge/dpc-music_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_drawline_ntsc() {
    run_self_test_on(
        "cartridge/dpc-drawline_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_drawline_pal() {
    run_self_test_on(
        "cartridge/dpc-drawline_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_drawline_secam() {
    run_self_test_on(
        "cartridge/dpc-drawline_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn dpc_probes_ntsc() {
    run_self_test_on(
        "cartridge/dpc-probes_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Dpc,
    );
}

#[test]
fn dpc_probes_pal() {
    run_self_test_on(
        "cartridge/dpc-probes_pal.a26",
        TvStandard::Pal,
        CartType::Dpc,
    );
}

#[test]
fn dpc_probes_secam() {
    run_self_test_on(
        "cartridge/dpc-probes_secam.a26",
        TvStandard::Secam,
        CartType::Dpc,
    );
}

#[test]
fn ar_config_ntsc() {
    run_self_test_on(
        "cartridge/ar-config_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Ar,
    );
}

#[test]
fn ar_config_pal() {
    run_self_test_on("cartridge/ar-config_pal.a26", TvStandard::Pal, CartType::Ar);
}

#[test]
fn ar_config_secam() {
    run_self_test_on(
        "cartridge/ar-config_secam.a26",
        TvStandard::Secam,
        CartType::Ar,
    );
}

#[test]
fn ar_write_ntsc() {
    run_self_test_on(
        "cartridge/ar-write_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Ar,
    );
}

#[test]
fn ar_write_pal() {
    run_self_test_on("cartridge/ar-write_pal.a26", TvStandard::Pal, CartType::Ar);
}

#[test]
fn ar_write_secam() {
    run_self_test_on(
        "cartridge/ar-write_secam.a26",
        TvStandard::Secam,
        CartType::Ar,
    );
}

#[test]
fn ar_hazards_ntsc() {
    run_self_test_on(
        "cartridge/ar-hazards_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Ar,
    );
}

#[test]
fn ar_hazards_pal() {
    run_self_test_on(
        "cartridge/ar-hazards_pal.a26",
        TvStandard::Pal,
        CartType::Ar,
    );
}

#[test]
fn ar_hazards_secam() {
    run_self_test_on(
        "cartridge/ar-hazards_secam.a26",
        TvStandard::Secam,
        CartType::Ar,
    );
}

#[test]
fn bank_f0_ntsc() {
    run_self_test_on("cartridge/bank-f0_ntsc.a26", TvStandard::Ntsc, CartType::F0);
}

#[test]
fn bank_f0_pal() {
    run_self_test_on("cartridge/bank-f0_pal.a26", TvStandard::Pal, CartType::F0);
}

#[test]
fn bank_f0_secam() {
    run_self_test_on(
        "cartridge/bank-f0_secam.a26",
        TvStandard::Secam,
        CartType::F0,
    );
}

#[test]
fn bank_wd_ntsc() {
    run_self_test_on("cartridge/bank-wd_ntsc.a26", TvStandard::Ntsc, CartType::Wd);
}

#[test]
fn bank_wd_pal() {
    run_self_test_on("cartridge/bank-wd_pal.a26", TvStandard::Pal, CartType::Wd);
}

#[test]
fn bank_wd_secam() {
    run_self_test_on(
        "cartridge/bank-wd_secam.a26",
        TvStandard::Secam,
        CartType::Wd,
    );
}

#[test]
fn bank_jane_ntsc() {
    run_self_test_on(
        "cartridge/bank-jane_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Jane,
    );
}

#[test]
fn bank_jane_pal() {
    run_self_test_on(
        "cartridge/bank-jane_pal.a26",
        TvStandard::Pal,
        CartType::Jane,
    );
}

#[test]
fn bank_jane_secam() {
    run_self_test_on(
        "cartridge/bank-jane_secam.a26",
        TvStandard::Secam,
        CartType::Jane,
    );
}

#[test]
fn bank_wf8_ntsc() {
    run_self_test_on(
        "cartridge/bank-wf8_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Wf8,
    );
}

#[test]
fn bank_wf8_pal() {
    run_self_test_on("cartridge/bank-wf8_pal.a26", TvStandard::Pal, CartType::Wf8);
}

#[test]
fn bank_wf8_secam() {
    run_self_test_on(
        "cartridge/bank-wf8_secam.a26",
        TvStandard::Secam,
        CartType::Wf8,
    );
}

#[test]
fn bank_fc_ntsc() {
    run_self_test_on("cartridge/bank-fc_ntsc.a26", TvStandard::Ntsc, CartType::Fc);
}

#[test]
fn bank_fc_pal() {
    run_self_test_on("cartridge/bank-fc_pal.a26", TvStandard::Pal, CartType::Fc);
}

#[test]
fn bank_fc_secam() {
    run_self_test_on(
        "cartridge/bank-fc_secam.a26",
        TvStandard::Secam,
        CartType::Fc,
    );
}

#[test]
fn bank_0fa0_ntsc() {
    run_self_test_on(
        "cartridge/bank-0fa0_ntsc.a26",
        TvStandard::Ntsc,
        CartType::ZeroFa0,
    );
}

#[test]
fn bank_0fa0_pal() {
    run_self_test_on(
        "cartridge/bank-0fa0_pal.a26",
        TvStandard::Pal,
        CartType::ZeroFa0,
    );
}

#[test]
fn bank_0fa0_secam() {
    run_self_test_on(
        "cartridge/bank-0fa0_secam.a26",
        TvStandard::Secam,
        CartType::ZeroFa0,
    );
}

#[test]
fn bank_03e0_ntsc() {
    run_self_test_on(
        "cartridge/bank-03e0_ntsc.a26",
        TvStandard::Ntsc,
        CartType::Zero3E0,
    );
}

#[test]
fn bank_03e0_pal() {
    run_self_test_on(
        "cartridge/bank-03e0_pal.a26",
        TvStandard::Pal,
        CartType::Zero3E0,
    );
}

#[test]
fn bank_03e0_secam() {
    run_self_test_on(
        "cartridge/bank-03e0_secam.a26",
        TvStandard::Secam,
        CartType::Zero3E0,
    );
}

#[test]
fn power_on_bank_ntsc() {
    run_self_test_on(
        "cartridge/power-on-bank_ntsc.a26",
        TvStandard::Ntsc,
        CartType::F8,
    );
}

#[test]
fn power_on_bank_pal() {
    run_self_test_on(
        "cartridge/power-on-bank_pal.a26",
        TvStandard::Pal,
        CartType::F8,
    );
}

#[test]
fn power_on_bank_secam() {
    run_self_test_on(
        "cartridge/power-on-bank_secam.a26",
        TvStandard::Secam,
        CartType::F8,
    );
}
