use crate::common;

#[test]
fn floating_bus_ntsc() {
    common::run_self_test("cpu/floating-bus_ntsc.a26");
}

#[test]
fn floating_bus_pal() {
    common::run_self_test("cpu/floating-bus_pal.a26");
}

#[test]
fn rmw_strobe_ntsc() {
    common::run_self_test("cpu/rmw-strobe_ntsc.a26");
}

#[test]
fn rmw_strobe_pal() {
    common::run_self_test("cpu/rmw-strobe_pal.a26");
}

#[test]
fn rmw_wsync_ntsc() {
    common::run_self_test("cpu/rmw-wsync_ntsc.a26");
}

#[test]
fn rmw_wsync_pal() {
    common::run_self_test("cpu/rmw-wsync_pal.a26");
}

#[test]
fn stack_aliases_ram_ntsc() {
    common::run_self_test("cpu/stack-aliases-ram_ntsc.a26");
}

#[test]
fn stack_aliases_ram_pal() {
    common::run_self_test("cpu/stack-aliases-ram_pal.a26");
}
