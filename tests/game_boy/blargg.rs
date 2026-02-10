use crate::common;

fn run_blargg_test(rom_path: &str) {
    run_blargg_test_with_timeout(rom_path, 3600);
}

fn run_blargg_test_with_timeout(rom_path: &str, timeout_frames: u32) {
    let mut gb = common::load_rom(rom_path);
    let output = common::run_until_serial_match(&mut gb, &["Passed", "Failed"], timeout_frames);
    assert!(
        output.contains("Passed"),
        "Blargg test {rom_path} failed. Serial output:\n{output}"
    );
}

// cpu_instrs — combined test (runs all 11 sub-tests, needs ~55s = ~3300 frames)
#[test]
fn cpu_instrs() {
    run_blargg_test_with_timeout("blargg/cpu_instrs/cpu_instrs.gb", 7200);
}

// cpu_instrs — individual sub-tests (faster to isolate failures)
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

// Instruction timing
#[test]
fn instr_timing() {
    run_blargg_test("blargg/instr_timing/instr_timing.gb");
}

// Memory timing
#[test]
fn mem_timing() {
    run_blargg_test("blargg/mem_timing/mem_timing.gb");
}

// Memory timing 2 (screen-only, no serial output)
#[test]
fn mem_timing_2() {
    run_blargg_test_with_timeout("blargg/mem_timing-2/mem_timing.gb", 1200);
}

// Halt bug (screen-only, no serial output)
#[test]
fn halt_bug() {
    run_blargg_test_with_timeout("blargg/halt_bug.gb", 1200);
}

// Interrupt timing (screen-only, no serial output)
#[test]
fn interrupt_time() {
    run_blargg_test_with_timeout("blargg/interrupt_time/interrupt_time.gb", 1200);
}

// DMG sound — combined (screen-only, no serial output)
#[test]
fn dmg_sound() {
    run_blargg_test_with_timeout("blargg/dmg_sound/dmg_sound.gb", 3600);
}

// OAM bug — combined (screen-only, no serial output)
#[test]
fn oam_bug() {
    run_blargg_test_with_timeout("blargg/oam_bug/oam_bug.gb", 1200);
}
