// One test per imported ROM; function names mirror the ROM stems (a
// leading underscore where the stem starts with a digit). The corpus runs
// twice, as an NTSC body (262 lines) and a PAL body (313 lines).

/// The body a corpus instance runs on, as the standard the VDP is cut for.
macro_rules! standard {
    (ntsc) => {
        missingno_ti_vdp::Standard::Ntsc
    };
    (pal) => {
        missingno_ti_vdp::Standard::Pal
    };
}

/// `ntsc_only` marks a cycle-counted ROM the corpus skips on a PAL SC-3000
/// (its Z80 is not locked to the VDP clock): it exists on the NTSC body alone.
macro_rules! vdp_test {
    (ntsc, $name:ident, $path:literal, ntsc_only $($rest:tt)*) => {
        vdp_test!(ntsc, $name, $path $($rest)*);
    };
    (pal, $name:ident, $path:literal, ntsc_only $($rest:tt)*) => {};
    ($body:ident, $name:ident, $path:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_pass($path, standard!($body));
        }
    };
    ($body:ident, $name:ident, $path:literal, frames = $frames:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_pass_within($path, standard!($body), $frames);
        }
    };
    ($body:ident, $name:ident, $path:literal, staged = $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            crate::testbench::assert_pass($path, standard!($body));
        }
    };
    ($body:ident, $name:ident, $path:literal, frames = $frames:literal, staged = $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            crate::testbench::assert_pass_within($path, standard!($body), $frames);
        }
    };
}

/// A screenshot subject: PASS latches once the scene is up, then the next
/// complete frame must match the blessed `_ntsc.png` reference exactly. A
/// scene whose picture on 313 lines is legitimately not the NTSC reference's
/// is `pal_staged`: it runs on NTSC and is staged on PAL alone.
macro_rules! vdp_screenshot {
    (ntsc, $name:ident, $path:literal, ntsc_only $($rest:tt)*) => {
        vdp_screenshot!(ntsc, $name, $path $($rest)*);
    };
    (pal, $name:ident, $path:literal, ntsc_only $($rest:tt)*) => {};
    ($body:ident, $name:ident, $path:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_screenshot($path, standard!($body));
        }
    };
    ($body:ident, $name:ident, $path:literal, staged = $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            crate::testbench::assert_screenshot($path, standard!($body));
        }
    };
    (ntsc, $name:ident, $path:literal, pal_staged = $reason:literal) => {
        #[test]
        fn $name() {
            crate::testbench::assert_screenshot($path, standard!(ntsc));
        }
    };
    (pal, $name:ident, $path:literal, pal_staged = $reason:literal) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            crate::testbench::assert_screenshot($path, standard!(pal));
        }
    };
}

/// The corpus, written once and instantiated per broadcast standard.
macro_rules! corpus {
    ($body:ident) => {
        mod harness {
            vdp_test!($body, sanity, "harness/sanity.sg");
            vdp_test!($body, port_mirror, "harness/port-mirror.sg");
            vdp_test!($body, ram_mirror, "harness/ram-mirror.sg");
            vdp_screenshot!($body, calibration, "harness/calibration.sg");
        }

        mod registers {
            // Anchor-development vehicle: a screenshot subject whose PASS latches
            // after two swept frames; the value here is the TRACE, which lets the
            // phase-anchor's F-edge sweep be debugged against a model that
            // reproduces the race (a racing status read swallows the flags).
            vdp_test!(
                $body,
                midline_name_sweep,
                "registers/midline-name-sweep.sg",
                ntsc_only
            );
            vdp_test!($body, midframe_base, "registers/midframe-base.sg");
            vdp_test!($body, midframe_blank, "registers/midframe-blank.sg");
            vdp_test!($body, midframe_size, "registers/midframe-size.sg");
            vdp_test!(
                $body,
                midframe_sprite_pattern,
                "registers/midframe-sprite-pattern.sg"
            );
            vdp_test!($body, reserved_bits, "registers/reserved-bits.sg");
            vdp_test!($body, select_mirror, "registers/select-mirror.sg");
            vdp_test!(
                $body,
                write_destroys_address,
                "registers/write-destroys-address.sg"
            );
            // Hardware-PRIMARY mid-frame scenes (see the modes note). The three
            // seam subjects stay staged: silicon shows sub-line effects (m2's seam
            // at column 8, backdrop's at 232, m1's disturbed transition row) that
            // the line-granular renderer cannot carry yet.
            vdp_screenshot!(
                $body,
                midframe_backdrop,
                "registers/midframe-backdrop.sg",
                staged = "seam is sub-line on silicon; line-granular renderer"
            );
            vdp_screenshot!(
                $body,
                midframe_m1,
                "registers/midframe-m1.sg",
                staged = "silicon steps at row 99 with a disturbed row 98; seam model open"
            );
            vdp_screenshot!(
                $body,
                midframe_m2,
                "registers/midframe-m2.sg",
                staged = "seam lands two cells late; whether mode bits latch at the fetch stage is open"
            );
            vdp_screenshot!($body, midframe_mask, "registers/midframe-mask.sg");
            vdp_screenshot!($body, midframe_mode, "registers/midframe-mode.sg");
            vdp_screenshot!($body, midframe_name, "registers/midframe-name.sg");
            // Sub-line seam subjects: the raster placement is calibrated against
            // midline-name's silicon seam (row 98, source column 16); these stay
            // staged until a capture-derived reference exists for the current
            // corpus builds.
            vdp_screenshot!(
                $body,
                midline_name,
                "registers/midline-name.sg",
                pal_staged = "the write pads 37835 T from the F edge, so on 313 lines the seam lands at row 47 (column 16 as on NTSC); no PAL reference"
            );
            vdp_screenshot!(
                $body,
                midline_backdrop,
                "registers/midline-backdrop.sg",
                ntsc_only,
                staged = "free-run build unphotographed; R7 drift validation vehicle"
            );
            // The midline walk family: one 229 T walk per scene, R7 as the
            // in-picture ruler. Hardware-PRIMARY (bless: never) and unphotographed.
            vdp_screenshot!(
                $body,
                midline_blank,
                "registers/midline-blank.sg",
                staged = "R1 blank walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_colour,
                "registers/midline-colour.sg",
                staged = "R3 walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_m1,
                "registers/midline-m1.sg",
                staged = "M1 walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_pattern,
                "registers/midline-pattern.sg",
                staged = "R4 walk unphotographed; hardware-PRIMARY, no reference"
            );
        }

        mod vram {
            vdp_test!($body, _4k_mode, "vram/4k-mode.sg");
            vdp_test!($body, access_windows, "vram/access-windows.sg");
            vdp_test!($body, addr_autoinc, "vram/addr-autoinc.sg");
            vdp_test!($body, addr_wrap, "vram/addr-wrap.sg");
            vdp_test!($body, drop_semantics, "vram/drop-semantics.sg");
            vdp_test!($body, latch_reset, "vram/latch-reset.sg");
            vdp_test!($body, read_ahead, "vram/read-ahead.sg");
            // Two 1800-frame retention waits: the sidecar budget is 4200 frames.
            vdp_test!($body, retention, "vram/retention.sg", frames = 4600);
            vdp_test!($body, undoc_retention, "vram/undoc-retention.sg");
        }

        mod status {
            vdp_test!($body, _5s_gating, "status/5s-gating.sg");
            vdp_test!($body, _5s_overwrite, "status/5s-overwrite.sg");
            vdp_test!($body, c_blank, "status/c-blank.sg");
            vdp_test!($body, c_gating, "status/c-gating.sg");
            vdp_test!($body, c_mag, "status/c-mag.sg");
            vdp_test!($body, _5s_relatch, "status/5s-relatch.sg");
            vdp_test!($body, fifth_sprite, "status/fifth-sprite.sg");
            vdp_test!($body, frame_flag, "status/frame-flag.sg");
            vdp_test!($body, status_clears, "status/status-clears.sg");
        }

        mod interrupt {
            vdp_test!($body, cadence, "interrupt/cadence.sg");
            vdp_test!($body, int_line, "interrupt/int-line.sg");
        }

        mod modes {
            vdp_screenshot!($body, backdrop_zero, "modes/backdrop-zero.sg");
            vdp_screenshot!($body, blank, "modes/blank.sg");
            vdp_screenshot!($body, gii_mask_pattern, "modes/gii-mask-pattern.sg");
            vdp_screenshot!($body, gii_shrunken, "modes/gii-shrunken.sg");
            vdp_screenshot!($body, graphic1, "modes/graphic1.sg");
            vdp_screenshot!($body, graphic2, "modes/graphic2.sg");
            vdp_screenshot!($body, multicolor, "modes/multicolor.sg");
            vdp_screenshot!($body, table_overlap, "modes/table-overlap.sg");
            // Hardware-PRIMARY subjects: consensus may not bless their references.
            // Where our frame matched the SC-3000 capture through the corpus's
            // arbiter, that frame is pinned as the reference; the rest stay staged
            // (an explicit run with TIVDP_DUMP_FRAMES dumps frames to adjudicate).
            vdp_screenshot!($body, text, "modes/text.sg");
            vdp_screenshot!($body, gii_mask_colour, "modes/gii-mask-colour.sg");
            vdp_screenshot!($body, undoc_bitmap_text, "modes/undoc-bitmap-text.sg");
            vdp_screenshot!(
                $body,
                undoc_text_multicolor,
                "modes/undoc-text-multicolor.sg"
            );
            vdp_screenshot!(
                $body,
                undoc_bitmap_multicolor,
                "modes/undoc-bitmap-multicolor.sg",
                staged = "closest candidate but unmatched residual; awaits the >=3px scene redesign"
            );
        }

        mod sprites {
            vdp_test!($body, coincidence, "sprites/coincidence.sg");
            vdp_test!($body, ec_geometry, "sprites/ec-geometry.sg");
            vdp_test!($body, edge_bleed, "sprites/edge-bleed.sg");
            vdp_test!($body, four_per_line, "sprites/four-per-line.sg");
            vdp_test!($body, mag_grid, "sprites/mag-grid.sg");
            vdp_test!($body, mode_gating, "sprites/mode-gating.sg");
            vdp_test!($body, name_mask, "sprites/name-mask.sg");
            vdp_test!($body, phantom_line, "sprites/phantom-line.sg");
            vdp_test!($body, size_mag, "sprites/size-mag.sg");
            vdp_test!($body, tag_bits, "sprites/tag-bits.sg");
            vdp_test!($body, terminator, "sprites/terminator.sg");
            vdp_test!($body, y_position, "sprites/y-position.sg");
            vdp_screenshot!($body, ghost, "sprites/ghost.sg");
            vdp_screenshot!($body, priority, "sprites/priority.sg");
            // Hardware-PRIMARY (bless: never): silicon drops late sprites in the
            // shrunken configuration with a graded, SAT-indexed gradient no
            // emulator renders; the anomaly is a stated model gap.
            vdp_screenshot!(
                $body,
                shrunken_dup,
                "sprites/shrunken-dup.sg",
                staged = "shrunken-table drop anomaly unmodelled; hardware-PRIMARY, no blessable reference"
            );
        }

        mod timing {
            vdp_test!($body, _4k_sweep, "timing/4k-sweep.sg", ntsc_only);
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!(
                $body,
                _5s_instant_low,
                "timing/5s-instant-low.sg",
                ntsc_only,
                frames = 1600
            );
            // Sidecar budget 1400; everything but the three run-boundary cells matches.
            vdp_test!(
                $body,
                _5s_instant_mid,
                "timing/5s-instant-mid.sg",
                ntsc_only,
                frames = 1400,
                staged = "three run-boundary cells read $00 on silicon at T68/T73/T74 under clear-then-probe, our texture 7/old/new (code 06); mechanism unattributed"
            );
            vdp_test!(
                $body,
                _5s_instant_high,
                "timing/5s-instant-high.sg",
                ntsc_only
            );
            vdp_test!($body, _5s_race, "timing/5s-race.sg", ntsc_only);
            vdp_test!($body, blank_burst, "timing/blank-burst.sg", ntsc_only);
            vdp_test!($body, border_burst, "timing/border-burst.sg", ntsc_only);
            vdp_test!($body, c_instant, "timing/c-instant.sg", ntsc_only);
            vdp_test!($body, c_instant_x, "timing/c-instant-x.sg", ntsc_only);
            vdp_test!($body, c_race, "timing/c-race.sg", ntsc_only);
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!(
                $body,
                cadence_4match,
                "timing/cadence-4match.sg",
                ntsc_only,
                frames = 1600
            );
            vdp_test!($body, cadence_8match, "timing/cadence-8match.sg", ntsc_only);
            vdp_test!($body, f_edge_locator, "timing/f-edge-locator.sg", ntsc_only);
            vdp_test!($body, f_race, "timing/f-race.sg", ntsc_only);
            vdp_test!(
                $body,
                gi_burst,
                "timing/gi-burst.sg",
                ntsc_only,
                frames = 3000
            );
            vdp_test!($body, gii_sweep, "timing/gii-sweep.sg", ntsc_only);
            vdp_test!($body, line0_sweep, "timing/line0-sweep.sg", ntsc_only);
            vdp_test!(
                $body,
                line187_sweep,
                "timing/line187-sweep.sg",
                ntsc_only,
                frames = 1400
            );
            vdp_test!($body, line96_sweep, "timing/line96-sweep.sg", ntsc_only);
            vdp_test!($body, m1_split_sweep, "timing/m1-split-sweep.sg", ntsc_only);
            vdp_test!($body, match_sweep, "timing/match-sweep.sg", ntsc_only);
            vdp_test!($body, mc_sweep, "timing/mc-sweep.sg", ntsc_only);
            vdp_test!($body, onset_blank, "timing/onset-blank.sg", ntsc_only);
            vdp_test!($body, onset_burst, "timing/onset-burst.sg", ntsc_only);
            vdp_test!($body, phantom_burst, "timing/phantom-burst.sg", ntsc_only);
            vdp_test!($body, phase_sweep, "timing/phase-sweep.sg", ntsc_only);
            vdp_test!($body, satkill_sweep, "timing/satkill-sweep.sg", ntsc_only);
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!(
                $body,
                scan_cadence,
                "timing/scan-cadence.sg",
                ntsc_only,
                frames = 1600
            );
            vdp_test!($body, steal_and, "timing/steal-and.sg", ntsc_only);
            vdp_test!($body, steal_prime, "timing/steal-prime.sg", ntsc_only);
            vdp_test!($body, steal_raw, "timing/steal-raw.sg", ntsc_only);
            vdp_test!($body, steal_sweep, "timing/steal-sweep.sg", ntsc_only);
            vdp_test!($body, steal15_raw, "timing/steal15-raw.sg", ntsc_only);
            vdp_test!($body, steal15_sweep, "timing/steal15-sweep.sg", ntsc_only);
            vdp_test!($body, term_cadence, "timing/term-cadence.sg", ntsc_only);
            vdp_test!($body, text_burst, "timing/text-burst.sg", ntsc_only);
            vdp_test!($body, turn_on_66, "timing/turn-on-66.sg", ntsc_only);
            vdp_test!(
                $body,
                undoc_all_sweep,
                "timing/undoc-all-sweep.sg",
                ntsc_only
            );
            vdp_test!(
                $body,
                undoc_bmc_sweep,
                "timing/undoc-bmc-sweep.sg",
                ntsc_only
            );
            vdp_test!($body, undoc_bt_sweep, "timing/undoc-bt-sweep.sg", ntsc_only);
            vdp_test!(
                $body,
                undoc_tmc_sweep,
                "timing/undoc-tmc-sweep.sg",
                ntsc_only
            );
        }
    };
}

mod testbench;

mod ntsc {
    corpus!(ntsc);
}

mod pal {
    corpus!(pal);
}
