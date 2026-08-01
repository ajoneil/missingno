//! Blargg test ROMs confirmed passing on CGB hardware.
//!
//! Limited to ROMs whose behaviour is identical on DMG and CGB at
//! single CPU speed: CPU instruction correctness, instruction timing,
//! memory access timing, and the HALT bug. DMG-only suites
//! (`dmg_sound`, `oam_bug`) are excluded — CGB has different APU
//! quirks and no OAM bug.

use crate::common;

fn run_blargg_test(rom_path: &str) {
    run_blargg_test_with_timeout(rom_path, 3600);
}

fn run_blargg_test_with_timeout(rom_path: &str, timeout_frames: u32) {
    let mut gbc = common::load_rom(rom_path);
    let output = common::run_until_serial_match(&mut gbc, &["Passed", "Failed"], timeout_frames);
    assert!(
        output.contains("Passed"),
        "Blargg test {rom_path} failed. Serial output:\n{output}"
    );
}

fn run_blargg_screen_test(rom_path: &str, reference_path: &str, timeout_frames: u32) {
    let mut gbc = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop(&mut gbc, timeout_frames);
    assert!(
        found_loop,
        "Blargg test {rom_path} timed out without reaching infinite loop"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_reference_png(reference_path);

    let mut mismatches = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if a != e {
            if mismatches < 10 {
                let (x, y) = (i % 160, i / 160);
                eprintln!("Pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(
        mismatches, 0,
        "Blargg test {rom_path}: {mismatches} pixel mismatches vs {reference_path}"
    );
}

#[test]
fn cpu_instrs() {
    run_blargg_test_with_timeout("blargg/cpu_instrs/cpu_instrs.gb", 7200);
}

#[test]
fn cpu_instrs_01_special() {
    run_blargg_test("blargg/cpu_instrs/individual/01-special.gb");
}

#[test]
fn cpu_instrs_02_interrupts() {
    run_blargg_test("blargg/cpu_instrs/individual/02-interrupts.gb");
}

#[test]
fn cpu_instrs_03_op_sp_hl() {
    run_blargg_test("blargg/cpu_instrs/individual/03-op sp,hl.gb");
}

#[test]
fn cpu_instrs_04_op_r_imm() {
    run_blargg_test("blargg/cpu_instrs/individual/04-op r,imm.gb");
}

#[test]
fn cpu_instrs_05_op_rp() {
    run_blargg_test("blargg/cpu_instrs/individual/05-op rp.gb");
}

#[test]
fn cpu_instrs_06_ld_r_r() {
    run_blargg_test("blargg/cpu_instrs/individual/06-ld r,r.gb");
}

#[test]
fn cpu_instrs_07_jr_jp_call_ret_rst() {
    run_blargg_test("blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb");
}

#[test]
fn cpu_instrs_08_misc_instrs() {
    run_blargg_test("blargg/cpu_instrs/individual/08-misc instrs.gb");
}

#[test]
fn cpu_instrs_09_op_r_r() {
    run_blargg_test("blargg/cpu_instrs/individual/09-op r,r.gb");
}

#[test]
fn cpu_instrs_10_bit_ops() {
    run_blargg_test("blargg/cpu_instrs/individual/10-bit ops.gb");
}

#[test]
fn cpu_instrs_11_op_a_hl() {
    run_blargg_test("blargg/cpu_instrs/individual/11-op a,(hl).gb");
}

#[test]
fn instr_timing() {
    run_blargg_test("blargg/instr_timing/instr_timing.gb");
}

#[test]
fn mem_timing() {
    run_blargg_test("blargg/mem_timing/mem_timing.gb");
}

#[test]
fn mem_timing_2() {
    run_blargg_screen_test(
        "blargg/mem_timing-2/mem_timing.gb",
        "blargg/mem_timing-2/mem_timing-dmg-cgb.png",
        1200,
    );
}

#[test]
fn halt_bug() {
    run_blargg_screen_test("blargg/halt_bug.gb", "blargg/halt_bug-dmg-cgb.png", 1200);
}

/// CGB-only ROM — fails on DMG hardware (per c-sp howto). Tests
/// interrupt timing in both normal and double-speed CPU modes; will
/// fail until KEY1 / double-speed lands.
#[test]
fn interrupt_time() {
    let mut gbc = common::load_cgb_rom("blargg/interrupt_time.gb");
    let found_loop = common::run_until_infinite_loop(&mut gbc, 1200);
    assert!(
        found_loop,
        "Blargg interrupt_time timed out without reaching infinite loop"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png("blargg/interrupt_time-cgb.png");

    let mut mismatches = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if a != e {
            if mismatches < 10 {
                let (x, y) = (i % 160, i / 160);
                eprintln!("Pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(
        mismatches, 0,
        "Blargg interrupt_time: {mismatches} pixel mismatches"
    );
}
