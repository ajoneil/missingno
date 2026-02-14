use crate::common;

const TIMEOUT_FRAMES: u32 = 7200;

fn run_samesuite_test(rom_path: &str) {
    let mut gb = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop(&mut gb, TIMEOUT_FRAMES);
    assert!(
        found_loop,
        "SameSuite test {rom_path} timed out without reaching infinite loop"
    );
    let cpu = gb.cpu();
    if !common::check_mooneye_pass(cpu) {
        let fib = [
            (cpu.b, 3, "B"),
            (cpu.c, 5, "C"),
            (cpu.d, 8, "D"),
            (cpu.e, 13, "E"),
            (cpu.h, 21, "H"),
            (cpu.l, 34, "L"),
        ];
        let passed = fib
            .iter()
            .take_while(|(val, expected, _)| val == expected)
            .count();
        let failed_reg = if passed < 6 { fib[passed].2 } else { "?" };
        let failed_val = if passed < 6 { fib[passed].0 } else { 0 };
        panic!(
            "SameSuite test {rom_path} failed at sub-test {} (register {failed_reg}=0x{failed_val:02X}, expected {}). \
             Registers: {}",
            passed + 1,
            if passed < 6 { fib[passed].1 } else { 0 },
            common::format_registers(cpu),
        );
    }
}

// apu/channel_3/ — wave RAM tests (DMG-compatible, no CGB double-speed)
#[test]
fn channel_3_wave_ram_dac_on_rw() {
    run_samesuite_test("samesuite/apu/channel_3/channel_3_wave_ram_dac_on_rw.gb");
}

#[test]
fn channel_3_wave_ram_locked_write() {
    run_samesuite_test("samesuite/apu/channel_3/channel_3_wave_ram_locked_write.gb");
}

// interrupt/ — interrupt edge cases (DMG-compatible)
#[test]
fn ei_delay_halt() {
    run_samesuite_test("samesuite/interrupt/ei_delay_halt.gb");
}
