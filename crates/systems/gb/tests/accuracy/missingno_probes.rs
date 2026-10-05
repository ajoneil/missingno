use crate::gbmicrotest::run_result_rom;

// A write-caused wrap in the M-cycle after a reload: NYDU, held reset by MUGY
// while MEXU loads, can't see it, so TIMA reads $00, as dmg-sim and the DMG
// schematics say. Confirmed on a DMG (a 1995 DMG-01, board DMG-CPU-08) for
// both the TAC and the DIV write, with both late controls reading $FF.
#[test]
fn timer_postreload_tac() {
    run_result_rom("missingno-probes/timer/timer_postreload_tac.gb");
}

#[test]
fn timer_postreload_div() {
    run_result_rom("missingno-probes/timer/timer_postreload_div.gb");
}

#[test]
fn timer_postreload_tac_late() {
    run_result_rom("missingno-probes/timer/timer_postreload_tac_late.gb");
}

#[test]
fn timer_postreload_div_late() {
    run_result_rom("missingno-probes/timer/timer_postreload_div_late.gb");
}
