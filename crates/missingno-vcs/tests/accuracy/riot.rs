use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn io_mirrors_ntsc() {
    common::run_self_test("riot/io-mirrors_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn io_mirrors_pal() {
    common::run_self_test("riot/io-mirrors_pal.a26", TvStandard::Pal);
}

#[test]
fn io_output_ntsc() {
    common::run_self_test("riot/io-output_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn io_output_pal() {
    common::run_self_test("riot/io-output_pal.a26", TvStandard::Pal);
}

#[test]
fn io_ports_ntsc() {
    common::run_self_test("riot/io-ports_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn io_ports_pal() {
    common::run_self_test("riot/io-ports_pal.a26", TvStandard::Pal);
}

#[test]
fn pa7_edge_ntsc() {
    common::run_self_test("riot/pa7-edge_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn pa7_edge_pal() {
    common::run_self_test("riot/pa7-edge_pal.a26", TvStandard::Pal);
}

#[test]
fn ram_ntsc() {
    common::run_self_test("riot/ram_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn ram_pal() {
    common::run_self_test("riot/ram_pal.a26", TvStandard::Pal);
}

#[test]
fn ram_mirrors_ntsc() {
    common::run_self_test("riot/ram-mirrors_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn ram_mirrors_pal() {
    common::run_self_test("riot/ram-mirrors_pal.a26", TvStandard::Pal);
}

#[test]
fn timer_ntsc() {
    common::run_self_test("riot/timer_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn timer_pal() {
    common::run_self_test("riot/timer_pal.a26", TvStandard::Pal);
}

#[test]
fn timer_divisors_ntsc() {
    common::run_self_test("riot/timer-divisors_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn timer_divisors_pal() {
    common::run_self_test("riot/timer-divisors_pal.a26", TvStandard::Pal);
}

#[test]
fn timer_vblank_spin_ntsc() {
    common::run_self_test("riot/timer-vblank-spin_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn timer_vblank_spin_pal() {
    common::run_self_test("riot/timer-vblank-spin_pal.a26", TvStandard::Pal);
}
