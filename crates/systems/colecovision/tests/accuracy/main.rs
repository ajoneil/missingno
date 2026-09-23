// One test per imported `.col` ROM, mirroring the ti-vdp crate's `.sg`
// harness entry for entry; function names mirror the ROM stems (a leading
// underscore where the stem starts with a digit). The corpus runs twice, on
// an NTSC console (262 lines) and a PAL console (313 lines).

/// The body a corpus instance runs on, as the standard the VDP is cut for.
macro_rules! standard {
    (ntsc) => {
        missingno_ti_vdp::Standard::Ntsc
    };
    (pal) => {
        missingno_ti_vdp::Standard::Pal
    };
}

macro_rules! vdp_test {
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
    ($body:ident, $name:ident, $path:literal, bus_only) => {
        #[test]
        fn $name() {
            crate::testbench::assert_bus_only($path, standard!($body));
        }
    };
}

/// A screenshot subject: PASS latches once the scene is up, then the next
/// complete frame must match the blessed `_ntsc.png` reference exactly. A
/// scene whose picture on 313 lines is legitimately not the NTSC reference's
/// is `pal_staged`: it runs on NTSC and is staged on PAL alone.
macro_rules! vdp_screenshot {
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
            vdp_test!($body, sanity, "harness/sanity.col");
            vdp_test!($body, port_mirror, "harness/port-mirror.col");
            vdp_test!($body, ram_mirror, "harness/ram-mirror.col");
            vdp_screenshot!($body, calibration, "harness/calibration.col");
        }

        mod registers {
            // Built on the phase anchor, which is cycle-counted for the SG-1000's
            // wait-state-free bus: on this board it latches the BUS ONLY skip.
            vdp_test!(
                $body,
                midline_name_sweep,
                "registers/midline-name-sweep.col",
                bus_only
            );
            vdp_test!($body, midframe_base, "registers/midframe-base.col");
            vdp_test!($body, midframe_blank, "registers/midframe-blank.col");
            vdp_test!($body, midframe_size, "registers/midframe-size.col");
            vdp_test!(
                $body,
                midframe_sprite_pattern,
                "registers/midframe-sprite-pattern.col"
            );
            vdp_test!($body, reserved_bits, "registers/reserved-bits.col");
            vdp_test!($body, select_mirror, "registers/select-mirror.col");
            vdp_test!(
                $body,
                write_destroys_address,
                "registers/write-destroys-address.col"
            );
            // Hardware-PRIMARY mid-frame scenes (see the modes note). The three
            // seam subjects stay staged: silicon shows sub-line effects (m2's seam
            // at column 8, backdrop's at 232, m1's disturbed transition row) that
            // the line-granular renderer cannot carry yet.
            vdp_screenshot!(
                $body,
                midframe_backdrop,
                "registers/midframe-backdrop.col",
                staged = "seam is sub-line on silicon; line-granular renderer"
            );
            vdp_screenshot!(
                $body,
                midframe_m1,
                "registers/midframe-m1.col",
                staged = "silicon steps at row 99 with a disturbed row 98; seam model open"
            );
            vdp_screenshot!(
                $body,
                midframe_m2,
                "registers/midframe-m2.col",
                staged = "seam lands two cells late; whether mode bits latch at the fetch stage is open"
            );
            vdp_screenshot!($body, midframe_mask, "registers/midframe-mask.col");
            vdp_screenshot!($body, midframe_mode, "registers/midframe-mode.col");
            vdp_screenshot!($body, midframe_name, "registers/midframe-name.col");
            // Sub-line seam subjects: the raster placement is calibrated against
            // midline-name's silicon seam (row 98, source column 16); these stay
            // staged until a capture-derived reference exists for the current
            // corpus builds.
            vdp_screenshot!(
                $body,
                midline_name,
                "registers/midline-name.col",
                staged = "hand-latched from the F edge on a ledger no ColecoVision capture has checked: the seam lands at row 98 column 88 on 262 lines and row 47 on 313; no ColecoVision reference"
            );
            vdp_screenshot!(
                $body,
                midline_backdrop,
                "registers/midline-backdrop.col",
                staged = "free-run build unphotographed; R7 drift validation vehicle; latches the SG-1000 BUS ONLY sentinel on this board"
            );
            // The midline walk family: one 229 T walk per scene, R7 as the
            // in-picture ruler. Hardware-PRIMARY (bless: never) and unphotographed.
            vdp_screenshot!(
                $body,
                midline_blank,
                "registers/midline-blank.col",
                staged = "R1 blank walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_colour,
                "registers/midline-colour.col",
                staged = "R3 walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_m1,
                "registers/midline-m1.col",
                staged = "M1 walk unphotographed; hardware-PRIMARY, no reference"
            );
            vdp_screenshot!(
                $body,
                midline_pattern,
                "registers/midline-pattern.col",
                staged = "R4 walk unphotographed; hardware-PRIMARY, no reference"
            );
        }

        mod vram {
            vdp_test!($body, _4k_mode, "vram/4k-mode.col");
            vdp_test!($body, access_windows, "vram/access-windows.col");
            vdp_test!($body, addr_autoinc, "vram/addr-autoinc.col");
            vdp_test!($body, addr_wrap, "vram/addr-wrap.col");
            vdp_test!($body, drop_semantics, "vram/drop-semantics.col");
            vdp_test!($body, latch_reset, "vram/latch-reset.col");
            vdp_test!($body, read_ahead, "vram/read-ahead.col");
            // Two 1800-frame retention waits: the sidecar budget is 4200 frames.
            vdp_test!($body, retention, "vram/retention.col", frames = 4600);
            vdp_test!($body, undoc_retention, "vram/undoc-retention.col");
        }

        mod status {
            vdp_test!($body, _5s_gating, "status/5s-gating.col");
            vdp_test!($body, _5s_overwrite, "status/5s-overwrite.col");
            vdp_test!($body, c_blank, "status/c-blank.col");
            vdp_test!($body, c_gating, "status/c-gating.col");
            vdp_test!($body, c_mag, "status/c-mag.col");
            vdp_test!($body, _5s_relatch, "status/5s-relatch.col");
            vdp_test!($body, fifth_sprite, "status/fifth-sprite.col");
            vdp_test!($body, frame_flag, "status/frame-flag.col");
            vdp_test!($body, status_clears, "status/status-clears.col");
        }

        mod interrupt {
            vdp_test!($body, cadence, "interrupt/cadence.col");
            vdp_test!($body, int_line, "interrupt/int-line.col");
        }

        mod modes {
            vdp_screenshot!($body, backdrop_zero, "modes/backdrop-zero.col");
            vdp_screenshot!($body, blank, "modes/blank.col");
            vdp_screenshot!($body, gii_mask_pattern, "modes/gii-mask-pattern.col");
            vdp_screenshot!($body, gii_shrunken, "modes/gii-shrunken.col");
            vdp_screenshot!($body, graphic1, "modes/graphic1.col");
            vdp_screenshot!($body, graphic2, "modes/graphic2.col");
            vdp_screenshot!($body, multicolor, "modes/multicolor.col");
            vdp_screenshot!($body, table_overlap, "modes/table-overlap.col");
            // Hardware-PRIMARY subjects: consensus may not bless their references.
            // Where our frame matched the SC-3000 capture through the corpus's
            // arbiter, that frame is pinned as the reference; the rest stay staged
            // (an explicit run with COLECOVISION_DUMP_FRAMES dumps frames to adjudicate).
            vdp_screenshot!($body, text, "modes/text.col");
            vdp_screenshot!($body, gii_mask_colour, "modes/gii-mask-colour.col");
            vdp_screenshot!($body, undoc_bitmap_text, "modes/undoc-bitmap-text.col");
            vdp_screenshot!($body, undoc_text_multicolor, "modes/undoc-text-multicolor.col");
            vdp_screenshot!(
                $body,
                undoc_bitmap_multicolor,
                "modes/undoc-bitmap-multicolor.col",
                staged = "closest candidate but unmatched residual; awaits the >=3px scene redesign"
            );
        }

        mod sprites {
            vdp_test!($body, coincidence, "sprites/coincidence.col");
            vdp_test!($body, ec_geometry, "sprites/ec-geometry.col");
            vdp_test!($body, edge_bleed, "sprites/edge-bleed.col");
            vdp_test!($body, four_per_line, "sprites/four-per-line.col");
            vdp_test!($body, mag_grid, "sprites/mag-grid.col");
            vdp_test!($body, mode_gating, "sprites/mode-gating.col");
            vdp_test!($body, name_mask, "sprites/name-mask.col");
            vdp_test!($body, phantom_line, "sprites/phantom-line.col");
            vdp_test!($body, size_mag, "sprites/size-mag.col");
            vdp_test!($body, tag_bits, "sprites/tag-bits.col");
            vdp_test!($body, terminator, "sprites/terminator.col");
            vdp_test!($body, y_position, "sprites/y-position.col");
            vdp_screenshot!($body, ghost, "sprites/ghost.col");
            vdp_screenshot!($body, priority, "sprites/priority.col");
            // Hardware-PRIMARY (bless: never): silicon drops late sprites in the
            // shrunken configuration with a graded, SAT-indexed gradient no
            // emulator renders; the anomaly is a stated model gap.
            vdp_screenshot!(
                $body,
                shrunken_dup,
                "sprites/shrunken-dup.col",
                staged = "shrunken-table drop anomaly unmodelled; hardware-PRIMARY, no blessable reference"
            );
        }

        mod timing {
            vdp_test!($body, _4k_sweep, "timing/4k-sweep.col");
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!($body, _5s_instant_low, "timing/5s-instant-low.col", frames = 1600);
            // Sidecar budget 1400; the .col build compiles the ruler check out.
            vdp_test!($body, _5s_instant_mid, "timing/5s-instant-mid.col", frames = 1400);
            vdp_test!($body, _5s_instant_high, "timing/5s-instant-high.col");
            vdp_test!($body, _5s_race, "timing/5s-race.col");
            vdp_test!($body, blank_burst, "timing/blank-burst.col");
            vdp_test!($body, border_burst, "timing/border-burst.col");
            vdp_test!($body, c_instant, "timing/c-instant.col");
            vdp_test!(
                $body,
                c_instant_x,
                "timing/c-instant-x.col",
                staged = "C latches at the line boundary; silicon sets it at the generating pixel (code 0E: counter $10 at the rise, silicon $17)"
            );
            vdp_test!($body, c_race, "timing/c-race.col");
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!($body, cadence_4match, "timing/cadence-4match.col", frames = 1600);
            vdp_test!($body, cadence_8match, "timing/cadence-8match.col");
            vdp_test!($body, f_edge_locator, "timing/f-edge-locator.col");
            vdp_test!($body, f_race, "timing/f-race.col");
            vdp_test!($body, gi_burst, "timing/gi-burst.col", frames = 3000);
            vdp_test!($body, gii_sweep, "timing/gii-sweep.col");
            vdp_test!($body, line0_sweep, "timing/line0-sweep.col");
            vdp_test!($body, line187_sweep, "timing/line187-sweep.col", frames = 1400);
            vdp_test!($body, line96_sweep, "timing/line96-sweep.col");
            vdp_test!($body, m1_split_sweep, "timing/m1-split-sweep.col");
            vdp_test!($body, match_sweep, "timing/match-sweep.col");
            vdp_test!($body, mc_sweep, "timing/mc-sweep.col");
            vdp_test!($body, onset_blank, "timing/onset-blank.col");
            vdp_test!($body, onset_burst, "timing/onset-burst.col");
            vdp_test!($body, phantom_burst, "timing/phantom-burst.col");
            vdp_test!($body, phase_sweep, "timing/phase-sweep.col");
            vdp_test!($body, satkill_sweep, "timing/satkill-sweep.col");
            // Sidecar budget 1400: ~550 frames of sweep + the per-cell map compare.
            vdp_test!($body, scan_cadence, "timing/scan-cadence.col", frames = 1600);
            vdp_test!($body, steal_and, "timing/steal-and.col");
            vdp_test!($body, steal_prime, "timing/steal-prime.col");
            vdp_test!($body, steal_raw, "timing/steal-raw.col");
            vdp_test!($body, steal_sweep, "timing/steal-sweep.col");
            vdp_test!($body, steal15_raw, "timing/steal15-raw.col");
            vdp_test!($body, steal15_sweep, "timing/steal15-sweep.col");
            vdp_test!($body, term_cadence, "timing/term-cadence.col");
            vdp_test!($body, text_burst, "timing/text-burst.col");
            vdp_test!(
                $body,
                turn_on_66,
                "timing/turn-on-66.col",
                staged = "schedule wakes three lines before display line 0; silicon's seam is 2.03-2.40 lines before it (code 07, c = -71)"
            );
            vdp_test!($body, undoc_all_sweep, "timing/undoc-all-sweep.col");
            vdp_test!($body, undoc_bmc_sweep, "timing/undoc-bmc-sweep.col");
            vdp_test!($body, undoc_bt_sweep, "timing/undoc-bt-sweep.col");
            vdp_test!($body, undoc_tmc_sweep, "timing/undoc-tmc-sweep.col");
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
