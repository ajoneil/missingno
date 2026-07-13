use crate::common;
use missingno_vcs::TvStandard;

#[test]
fn floating_bus_ntsc() {
    common::run_self_test("cpu/floating-bus_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn floating_bus_pal() {
    common::run_self_test("cpu/floating-bus_pal.a26", TvStandard::Pal);
}

#[test]
fn floating_bus_secam() {
    common::run_self_test("cpu/floating-bus_secam.a26", TvStandard::Secam);
}

#[test]
fn partial_drive_ntsc() {
    common::run_self_test("cpu/partial-drive_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn partial_drive_pal() {
    common::run_self_test("cpu/partial-drive_pal.a26", TvStandard::Pal);
}

#[test]
fn partial_drive_secam() {
    common::run_self_test("cpu/partial-drive_secam.a26", TvStandard::Secam);
}

#[test]
fn rmw_strobe_ntsc() {
    common::run_self_test("cpu/rmw-strobe_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn rmw_strobe_pal() {
    common::run_self_test("cpu/rmw-strobe_pal.a26", TvStandard::Pal);
}

#[test]
fn rmw_strobe_secam() {
    common::run_self_test("cpu/rmw-strobe_secam.a26", TvStandard::Secam);
}

#[test]
fn rmw_wsync_ntsc() {
    common::run_self_test("cpu/rmw-wsync_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn rmw_wsync_pal() {
    common::run_self_test("cpu/rmw-wsync_pal.a26", TvStandard::Pal);
}

#[test]
fn rmw_wsync_secam() {
    common::run_self_test("cpu/rmw-wsync_secam.a26", TvStandard::Secam);
}

#[test]
fn stack_aliases_ram_ntsc() {
    common::run_self_test("cpu/stack-aliases-ram_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn stack_aliases_ram_pal() {
    common::run_self_test("cpu/stack-aliases-ram_pal.a26", TvStandard::Pal);
}

#[test]
fn stack_aliases_ram_secam() {
    common::run_self_test("cpu/stack-aliases-ram_secam.a26", TvStandard::Secam);
}

#[test]
fn stack_aliases_tia_ntsc() {
    common::run_self_test("cpu/stack-aliases-tia_ntsc.a26", TvStandard::Ntsc);
}

#[test]
fn stack_aliases_tia_pal() {
    common::run_self_test("cpu/stack-aliases-tia_pal.a26", TvStandard::Pal);
}

#[test]
fn stack_aliases_tia_secam() {
    common::run_self_test("cpu/stack-aliases-tia_secam.a26", TvStandard::Secam);
}
