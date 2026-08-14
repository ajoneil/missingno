// One test per imported ROM; function names mirror the ROM stems (a
// leading underscore where the stem starts with a digit).

macro_rules! vdp_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_pass($path);
        }
    };
    ($name:ident, $path:literal, frames = $frames:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_pass_within($path, $frames);
        }
    };
    ($name:ident, $path:literal, staged = $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            crate::testbench::assert_pass($path);
        }
    };
}

mod testbench;

mod harness {
    vdp_test!(sanity, "harness/sanity.sg");
    vdp_test!(port_mirror, "harness/port-mirror.sg");
    vdp_test!(ram_mirror, "harness/ram-mirror.sg");
}

mod registers {
    // Anchor-development vehicle: a screenshot subject whose PASS latches
    // after two swept frames; the value here is the TRACE, which lets the
    // phase-anchor's F-edge sweep be debugged against a model that
    // reproduces the race (a racing status read swallows the flags).
    vdp_test!(midline_name_sweep, "registers/midline-name-sweep.sg");
    vdp_test!(midframe_base, "registers/midframe-base.sg");
    vdp_test!(midframe_blank, "registers/midframe-blank.sg");
    vdp_test!(midframe_size, "registers/midframe-size.sg");
    vdp_test!(
        midframe_sprite_pattern,
        "registers/midframe-sprite-pattern.sg"
    );
    vdp_test!(reserved_bits, "registers/reserved-bits.sg");
    vdp_test!(select_mirror, "registers/select-mirror.sg");
    vdp_test!(
        write_destroys_address,
        "registers/write-destroys-address.sg"
    );
}

mod vram {
    vdp_test!(_4k_mode, "vram/4k-mode.sg");
    vdp_test!(access_windows, "vram/access-windows.sg");
    vdp_test!(addr_autoinc, "vram/addr-autoinc.sg");
    vdp_test!(addr_wrap, "vram/addr-wrap.sg");
    vdp_test!(drop_semantics, "vram/drop-semantics.sg");
    vdp_test!(latch_reset, "vram/latch-reset.sg");
    vdp_test!(read_ahead, "vram/read-ahead.sg");
    // Two 1800-frame retention waits: the sidecar budget is 4200 frames.
    vdp_test!(retention, "vram/retention.sg", frames = 4600);
    vdp_test!(undoc_retention, "vram/undoc-retention.sg");
}

mod status {
    vdp_test!(_5s_gating, "status/5s-gating.sg");
    vdp_test!(_5s_overwrite, "status/5s-overwrite.sg");
    vdp_test!(c_blank, "status/c-blank.sg");
    vdp_test!(c_gating, "status/c-gating.sg");
    vdp_test!(c_mag, "status/c-mag.sg");
    vdp_test!(_5s_relatch, "status/5s-relatch.sg");
    vdp_test!(fifth_sprite, "status/fifth-sprite.sg");
    vdp_test!(frame_flag, "status/frame-flag.sg");
    vdp_test!(status_clears, "status/status-clears.sg");
}

mod interrupt {
    vdp_test!(cadence, "interrupt/cadence.sg");
    vdp_test!(int_line, "interrupt/int-line.sg");
}

mod sprites {
    vdp_test!(coincidence, "sprites/coincidence.sg");
    vdp_test!(ec_geometry, "sprites/ec-geometry.sg");
    vdp_test!(edge_bleed, "sprites/edge-bleed.sg");
    vdp_test!(four_per_line, "sprites/four-per-line.sg");
    vdp_test!(mag_grid, "sprites/mag-grid.sg");
    vdp_test!(mode_gating, "sprites/mode-gating.sg");
    vdp_test!(name_mask, "sprites/name-mask.sg");
    vdp_test!(phantom_line, "sprites/phantom-line.sg");
    vdp_test!(size_mag, "sprites/size-mag.sg");
    vdp_test!(tag_bits, "sprites/tag-bits.sg");
    vdp_test!(terminator, "sprites/terminator.sg");
    vdp_test!(y_position, "sprites/y-position.sg");
}

mod timing {
    vdp_test!(_4k_sweep, "timing/4k-sweep.sg");
    vdp_test!(_5s_race, "timing/5s-race.sg");
    vdp_test!(blank_burst, "timing/blank-burst.sg");
    vdp_test!(blank_sweep, "timing/blank-sweep.sg");
    vdp_test!(border_burst, "timing/border-burst.sg");
    vdp_test!(border_sweep, "timing/border-sweep.sg");
    vdp_test!(c_race, "timing/c-race.sg");
    // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
    vdp_test!(cadence_4match, "timing/cadence-4match.sg", frames = 1600);
    vdp_test!(cadence_8match, "timing/cadence-8match.sg");
    vdp_test!(f_race, "timing/f-race.sg");
    vdp_test!(gi_burst, "timing/gi-burst.sg");
    vdp_test!(gii_sweep, "timing/gii-sweep.sg");
    vdp_test!(line0_sweep, "timing/line0-sweep.sg");
    vdp_test!(line187_sweep, "timing/line187-sweep.sg");
    vdp_test!(line96_sweep, "timing/line96-sweep.sg");
    vdp_test!(m1_split_sweep, "timing/m1-split-sweep.sg");
    vdp_test!(match_sweep, "timing/match-sweep.sg");
    vdp_test!(mc_sweep, "timing/mc-sweep.sg");
    vdp_test!(onset_burst, "timing/onset-burst.sg");
    vdp_test!(phantom_burst, "timing/phantom-burst.sg");
    vdp_test!(phase_sweep, "timing/phase-sweep.sg");
    vdp_test!(satkill_sweep, "timing/satkill-sweep.sg");
    // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
    vdp_test!(scan_cadence, "timing/scan-cadence.sg", frames = 1600);
    vdp_test!(steal_raw, "timing/steal-raw.sg");
    vdp_test!(steal_sweep, "timing/steal-sweep.sg");
    vdp_test!(steal15_sweep, "timing/steal15-sweep.sg");
    vdp_test!(text_burst, "timing/text-burst.sg");
    vdp_test!(text_sweep, "timing/text-sweep.sg");
    vdp_test!(undoc_all_sweep, "timing/undoc-all-sweep.sg");
    vdp_test!(undoc_bmc_sweep, "timing/undoc-bmc-sweep.sg");
    vdp_test!(undoc_bt_sweep, "timing/undoc-bt-sweep.sg");
    vdp_test!(undoc_tmc_sweep, "timing/undoc-tmc-sweep.sg");
}
