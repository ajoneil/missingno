//! Shared test helpers for accuracy/integration tests.
//!
//! Enabled by the `test-support` feature. Exposes a [`System`] trait
//! implemented by both `GameBoy` and downstream system crates (e.g.
//! `missingno-gbc`'s `GameBoyColor`), and runner/utility functions
//! generic over that trait so test ROMs and helpers can be reused
//! across systems.
//!
//! ROM paths are resolved relative to this crate's `CARGO_MANIFEST_DIR`,
//! so downstream crates can call [`rom_path`] / [`load_rom`] and pick
//! up ROMs from `crates/systems/gb/tests/accuracy/roms/`.

use std::path::{Path, PathBuf};

use crate::{
    BootRom, Console, GameBoy, Model, ScreenBuffer, cartridge::Cartridge, cpu::Cpu,
    execute::StepResult, interrupts,
};

use crate::system::ConsoleUi;

use missingno_test_support::compare::{assert_pixels_match, hex_byte};
use missingno_test_support::reference::ReferencePng;

#[cfg(feature = "morepork")]
use crate::trace::{TraceScope, Tracer};

/// Common interface for a Game Boy–family console runnable under the
/// shared accuracy test helpers. Implemented by `GameBoy` here and by
/// downstream systems (e.g. `GameBoyColor`).
///
/// The screen crosses this seam only as greyscale bytes — the DMG stores
/// 2-bit shade indices and the CGB stores colours, so a caller comparing
/// against a colour reference goes through the concrete type.
pub trait System {
    fn step(&mut self) -> StepResult;
    fn read(&self, address: u16) -> u8;
    fn cpu(&self) -> &Cpu;
    fn drain_serial_output(&mut self) -> Vec<u8>;
    fn interrupts(&self) -> &interrupts::Registers;
    /// True while a CGB double-speed switch holds the CPU stopped in its
    /// settling blackout (a self-resuming STOP).
    fn speed_switch_in_progress(&self) -> bool;
    /// True while a VRAM DMA holds the CPU (the bus master's hold, not a
    /// software STOP/HALT).
    fn vram_dma_holds_cpu(&self) -> bool;
    /// Peek a contiguous range of memory, bypassing bus conflicts and
    /// PPU mode gating. Used by tests that decode assertion records
    /// from WRAM after the test has halted.
    fn peek_range(&self, start: u16, len: u16) -> Vec<u8>;
    /// Drain accumulated audio samples (stereo, f32 pairs). Used by
    /// gambatte audio tests to check whether the test ROM produced
    /// any sound (`_outaudio1`) or was silent (`_outaudio0`).
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    /// The displayed screen as flat greyscale bytes, on the DMG reference
    /// shade ramp the shade-pattern references compare against.
    fn screen_greyscale(&self) -> Vec<u8>;
}

impl<M: Model> System for Console<M> {
    fn step(&mut self) -> StepResult {
        Console::<M>::step(self)
    }
    fn read(&self, address: u16) -> u8 {
        Console::<M>::read(self, address)
    }
    fn cpu(&self) -> &Cpu {
        Console::<M>::cpu(self)
    }
    fn drain_serial_output(&mut self) -> Vec<u8> {
        Console::<M>::drain_serial_output(self)
    }
    fn interrupts(&self) -> &interrupts::Registers {
        Console::<M>::interrupts(self)
    }
    fn speed_switch_in_progress(&self) -> bool {
        Console::<M>::speed_switch_in_progress(self)
    }
    fn vram_dma_holds_cpu(&self) -> bool {
        Console::<M>::vram_dma_holds_cpu(self)
    }
    fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        Console::<M>::peek_range(self, start, len)
    }
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        Console::<M>::drain_audio_samples(self)
    }
    fn screen_greyscale(&self) -> Vec<u8> {
        self.screen().to_greyscale_bytes()
    }
}

pub fn rom_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(relative)
}

/// A reference PNG from the shared roms tree, as one shade byte per pixel.
pub fn load_reference_png(relative: &str) -> Vec<u8> {
    ReferencePng::load(&rom_path(relative)).greyscale()
}

/// Compare a rendered screen against the greyscale reference PNG at `path`.
pub fn assert_screen_matches_png(subject: &str, screen: &[u8], path: &Path) {
    let expected = ReferencePng::load(path).greyscale();
    assert_pixels_match(subject, screen, &expected, 160, 10, hex_byte);
}

/// Compare a rendered screen against a reference PNG in the shared roms tree.
pub fn assert_screen_matches(subject: &str, screen: &[u8], reference: &str) {
    assert_screen_matches_png(subject, screen, &rom_path(reference));
}

/// A test run wrapping a `GameBoy` and an optional trace writer.
///
/// When the `morepork` feature is enabled and the `MOREPORK_PROFILE` env var
/// is set (any value enables capture — the column set comes from the state
/// schema, not a named profile), each `step()` captures state into a native
/// `.morepork` trace file under `receipts/traces/`.
pub struct TestRun<M: Model> {
    pub gb: Console<M>,
    #[cfg(feature = "morepork")]
    tracer: TracerGuard,
}

/// Owns the run's tracer so an abandoned run still flushes its trace on drop,
/// without giving `TestRun` itself a `Drop` impl (tests move `gb` out of it).
#[cfg(feature = "morepork")]
struct TracerGuard(Option<Tracer>);

#[cfg(feature = "morepork")]
impl Drop for TracerGuard {
    fn drop(&mut self) {
        if let Some(tracer) = self.0.take() {
            let _ = tracer.finish();
        }
    }
}

impl<M: ConsoleUi> TestRun<M> {
    /// Wrap a console for a traced run. `model_label` is the hardware-model
    /// string written into the trace metadata (e.g. "DMG-B", "CGB-C").
    pub fn new(gb: Console<M>, _rom_relative: &str, _model_label: &str) -> Self {
        #[cfg(feature = "morepork")]
        let tracer = TracerGuard(try_create_tracer(&gb, _rom_relative, _model_label));

        Self {
            gb,
            #[cfg(feature = "morepork")]
            tracer,
        }
    }

    /// Step one instruction, capturing trace state if active.
    ///
    /// For tcycle-triggered profiles, this steps dot-by-dot and captures
    /// state at every T-cycle. For instruction-triggered profiles, it
    /// captures once before the instruction executes.
    pub fn step(&mut self) -> StepResult {
        #[cfg(feature = "morepork")]
        {
            if let Some(tracer) = &mut self.tracer.0 {
                if tracer.trigger() == crate::trace::Trigger::Tcycle {
                    return self.step_traced_tcycle();
                }
                self.gb.sync_audio();
                self.gb.sync_ppu();
                tracer.capture(&self.gb).unwrap();
            }

            let result = self.gb.step();

            if let Some(tracer) = &mut self.tracer.0 {
                tracer.advance(result.tcycles);
                if result.new_screen {
                    tracer.mark_frame().unwrap();
                }
            }

            result
        }

        #[cfg(not(feature = "morepork"))]
        self.gb.step()
    }

    /// Step one instruction by advancing one dot at a time, capturing at each dot.
    #[cfg(feature = "morepork")]
    fn step_traced_tcycle(&mut self) -> StepResult {
        crate::trace::step_instruction_tcycle(&mut self.gb, self.tracer.0.as_mut().unwrap())
    }

    #[allow(unused_mut)]
    pub fn finish(mut self) {
        #[cfg(feature = "morepork")]
        if let Some(tracer) = self.tracer.0.take() {
            tracer.finish().unwrap();
        }
    }
}

impl<M: ConsoleUi> System for TestRun<M> {
    fn step(&mut self) -> StepResult {
        TestRun::step(self)
    }
    fn read(&self, address: u16) -> u8 {
        self.gb.read(address)
    }
    fn cpu(&self) -> &Cpu {
        self.gb.cpu()
    }
    fn drain_serial_output(&mut self) -> Vec<u8> {
        self.gb.drain_serial_output()
    }
    fn interrupts(&self) -> &interrupts::Registers {
        self.gb.interrupts()
    }
    fn speed_switch_in_progress(&self) -> bool {
        self.gb.speed_switch_in_progress()
    }
    fn vram_dma_holds_cpu(&self) -> bool {
        self.gb.vram_dma_holds_cpu()
    }
    fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        self.gb.peek_range(start, len)
    }
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.gb.drain_audio_samples()
    }
    fn screen_greyscale(&self) -> Vec<u8> {
        self.gb.screen_greyscale()
    }
}

/// Build a schema-driven tracer when capture is requested. Capture is enabled by
/// `MOREPORK_PROFILE` (any value, kept for muscle memory); `MOREPORK_TRIGGER`
/// (`tcycle`/`instruction`, default instruction) sets the cadence and
/// `MOREPORK_SCOPE` (`full`/`observable`, default observable) the tier depth. The
/// column set and its typing come from the console model's state schema.
#[cfg(feature = "morepork")]
fn try_create_tracer<M: ConsoleUi>(
    gb: &Console<M>,
    rom_relative: &str,
    model_label: &str,
) -> Option<Tracer> {
    std::env::var("MOREPORK_PROFILE").ok()?;

    // Unset falls back to the documented default; an explicit but unrecognized
    // value is a harness misconfiguration, not a silent default — fail loudly.
    let trigger = match std::env::var("MOREPORK_TRIGGER").as_deref() {
        Ok("tcycle") => crate::trace::Trigger::Tcycle,
        Ok("instruction") => crate::trace::Trigger::Instruction,
        Err(_) => crate::trace::Trigger::Instruction,
        Ok(other) => {
            panic!("MOREPORK_TRIGGER: unknown value {other:?} (expected `tcycle` or `instruction`)")
        }
    };
    let scope = match std::env::var("MOREPORK_SCOPE").as_deref() {
        Ok("full") => TraceScope::Full,
        Ok("observable") => TraceScope::Observable,
        Err(_) => TraceScope::Observable,
        Ok(other) => {
            panic!("MOREPORK_SCOPE: unknown value {other:?} (expected `full` or `observable`)")
        }
    };

    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../receipts/traces");
    std::fs::create_dir_all(&output_dir).unwrap();

    let rom_stem = Path::new(rom_relative)
        .file_stem()
        .unwrap()
        .to_string_lossy();
    let output_path = output_dir.join(format!("{rom_stem}.morepork"));

    eprintln!("morepork: writing {}", output_path.display());

    let mut tracer = Tracer::create(
        &output_path,
        gb,
        trigger,
        scope,
        crate::trace::BootRom::Skip,
        model_label,
    )
    .unwrap_or_else(|e| panic!("Failed to create tracer: {e}"));

    tracer.mark_frame().unwrap();

    Some(tracer)
}

pub fn load_rom(relative: &str) -> TestRun<crate::Dmg> {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    let boot_rom = try_load_boot_rom();
    let mut gb = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), boot_rom);
    run_boot_rom(&mut gb);
    TestRun::new(gb, relative, "DMG-B")
}

/// Try to load the DMG boot ROM from the path in `DMG_BOOT_ROM`.
/// Returns None if the env var is unset or the file can't be read.
/// The boot ROM cannot be distributed with the repo for legal reasons.
fn try_load_boot_rom() -> Option<BootRom> {
    let path = std::env::var("DMG_BOOT_ROM").ok()?;
    let data = std::fs::read(&path).ok()?;
    let boxed: Box<[u8; 256]> = data.into_boxed_slice().try_into().ok()?;
    Some(BootRom::Dmg(boxed))
}

/// Drive a mapped boot ROM to the cartridge handoff at PC=0x0100. A no-op
/// when no boot ROM is mapped (the CPU is already post-boot at 0x0100).
pub fn run_boot_rom<M: Model>(gb: &mut Console<M>) {
    if gb.cpu().ir_address != 0x0000 {
        return;
    }
    for _ in 0..10_000_000 {
        gb.step();
        if gb.cpu().ir_address == 0x0100 {
            return;
        }
    }
    panic!(
        "Boot ROM did not reach 0x0100 within 10M steps — does the ROM have a valid Nintendo logo?"
    );
}

/// Run the emulator until the serial output contains any of the given needle strings,
/// or until an infinite loop is detected at a frame boundary, or until a timeout is reached.
pub fn run_until_serial_match<S: System>(
    s: &mut S,
    needles: &[&str],
    timeout_frames: u32,
) -> String {
    let mut output = String::new();
    for _ in 0..timeout_frames {
        while !s.step().new_screen {}
        let bytes = s.drain_serial_output();
        if !bytes.is_empty() {
            output.push_str(&String::from_utf8_lossy(&bytes));
            if needles.iter().any(|needle| output.contains(needle)) {
                return output;
            }
        }
        if is_infinite_loop(s) {
            return output;
        }
    }
    output
}

/// Run the emulator for a fixed number of frames.
pub fn run_frames<S: System>(s: &mut S, frames: u32) {
    for _ in 0..frames {
        while !s.step().new_screen {}
    }
}

/// Run the emulator for a fixed number of T-cycles. Unlike
/// [`run_frames`], doesn't depend on the LCD producing frames — used
/// by gambatte tests which finish after a fixed cycle count (the
/// gambatte testrunner runs for 1,053,360 T-cycles, equal to 15 LCD
/// frames at single speed).
pub fn run_for_tcycles<S: System>(s: &mut S, max_tcycles: u32) {
    let mut total: u32 = 0;
    while total < max_tcycles {
        let result = s.step();
        total = total.saturating_add(result.tcycles);
    }
}

/// Run the emulator until it enters an infinite loop, or until a timeout (in frames) is reached.
pub fn run_until_infinite_loop<S: System>(s: &mut S, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        while !s.step().new_screen {}
        if is_infinite_loop(s) {
            return true;
        }
    }
    // Per-instruction scan for HALT-based completion loops
    for _ in 0..70224 {
        s.step();
        if is_infinite_loop(s) {
            return true;
        }
    }
    false
}

/// Run the emulator until `LD B,B` (opcode 0x40) is about to execute in
/// ROM/WRAM, or until a timeout.
pub fn run_until_breakpoint<S: System>(s: &mut S, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        loop {
            let pc = s.cpu().ir_address;
            // A 0x40 fetched from I/O space (e.g. DIV during `call rDIV`) is the
            // register value being executed, not the LD B,B completion marker.
            if pc < 0xFF00 && s.read(pc) == 0x40 {
                return true;
            }
            if s.step().new_screen {
                break;
            }
        }
    }
    false
}

/// Run the emulator until opcode 0xED (undefined) is about to execute, or until
/// an infinite loop is detected, or until a timeout.
pub fn run_until_undefined_opcode<S: System>(s: &mut S, timeout_frames: u32) -> bool {
    for _ in 0..timeout_frames {
        loop {
            let pc = s.cpu().ir_address;
            if s.read(pc) == 0xED {
                return true;
            }
            if is_infinite_loop(s) {
                return true;
            }
            if s.step().new_screen {
                break;
            }
        }
    }
    false
}

/// Run the emulator instruction-by-instruction (no LCD frame
/// assumption) until an infinite loop is detected or the instruction
/// budget is exhausted.
///
/// Use this for tests that don't enable the LCD — frame-based runners
/// hang in the inner `while !step().new_screen {}` loop when no frame
/// is ever produced.
pub fn run_until_infinite_loop_no_lcd<S: System>(s: &mut S, max_instructions: u32) -> bool {
    for _ in 0..max_instructions {
        s.step();
        if is_infinite_loop(s) {
            return true;
        }
    }
    false
}

/// Check if the CPU is stuck in a known completion loop.
pub fn is_infinite_loop<S: System>(s: &S) -> bool {
    let pc = s.cpu().ir_address;
    if s.read(pc) == 0x18 && s.read(pc.wrapping_add(1)) == 0xFE {
        return true;
    }
    if s.read(pc.wrapping_sub(1)) == 0x18 && s.read(pc) == 0xFE {
        return true;
    }
    if s.read(pc) == 0x40
        && s.read(pc.wrapping_add(1)) == 0x18
        && s.read(pc.wrapping_add(2)) == 0xFE
    {
        return true;
    }

    if s.cpu().halt.state != crate::cpu::HaltState::Running
        && !s.speed_switch_in_progress()
        && !s.vram_dma_holds_cpu()
    {
        if s.interrupts().enabled.is_empty() {
            return true;
        }

        if s.read(pc.wrapping_sub(1)) == 0x76 {
            for offset in 0u16..4 {
                let addr = pc.wrapping_add(offset);
                if s.read(addr) == 0x18 {
                    let rel = s.read(addr.wrapping_add(1)) as i8;
                    let target = addr.wrapping_add(2).wrapping_add(rel as u16);
                    if target <= pc.wrapping_sub(1) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn check_mooneye_pass(cpu: &Cpu) -> bool {
    cpu.b == 3 && cpu.c == 5 && cpu.d == 8 && cpu.e == 13 && cpu.h == 21 && cpu.l == 34
}

pub fn format_registers(cpu: &Cpu) -> String {
    format!(
        "B={} C={} D={} E={} H={} L={} (expected: B=3 C=5 D=8 E=13 H=21 L=34)",
        cpu.b, cpu.c, cpu.d, cpu.e, cpu.h, cpu.l
    )
}

pub fn format_wram_dump<S: System>(s: &S, start: u16, len: u16) -> String {
    let mut out = String::new();
    let mut offset: u16 = 0;
    while offset < len {
        let row_addr = start.wrapping_add(offset);
        out.push_str(&format!("\n  ${row_addr:04X}:"));
        for i in 0..16 {
            if offset + i >= len {
                break;
            }
            out.push_str(&format!(" {:02X}", s.read(row_addr.wrapping_add(i))));
        }
        offset = offset.wrapping_add(16);
    }
    out
}

/// Drive a Mooneye ROM to its completion loop and require the Fibonacci
/// pass registers, reporting which sub-test the register walk stalled at.
pub fn assert_mooneye_verdict<S: System>(s: &mut S, rom_path: &str, timeout_frames: u32) {
    let mut serial_output = String::new();
    let found_loop = run_until_infinite_loop(s, timeout_frames);
    let bytes = s.drain_serial_output();
    if !bytes.is_empty() {
        serial_output.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(
        found_loop,
        "Mooneye test {rom_path} timed out without reaching infinite loop"
    );
    let cpu = s.cpu();
    if check_mooneye_pass(cpu) {
        return;
    }

    // Most Mooneye tests set registers to Fibonacci values (3,5,8,13,21,34) in
    // order as sub-tests pass. Some (e.g. lcdon_timing-GS) use quit_inline,
    // which sets ALL registers to 0x42 on any failure — detect that pattern so
    // the report doesn't misattribute the failure to sub-test 1.
    let all_same =
        cpu.b == cpu.c && cpu.c == cpu.d && cpu.d == cpu.e && cpu.e == cpu.h && cpu.h == cpu.l;
    if all_same && cpu.b != 0 {
        panic!(
            "Mooneye test {rom_path} failed (all registers = 0x{:02X}, ROM uses \
             uniform failure — sub-test number unknown). Serial: {:?}",
            cpu.b, serial_output,
        );
    }

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
    eprintln!(
        "Sub-tests passed: {passed}/6 (failed at register {failed_reg}, got 0x{failed_val:02X})"
    );
    panic!(
        "Mooneye test {rom_path} failed at sub-test {} (register {failed_reg}=0x{failed_val:02X}, expected {}). \
         Registers: {} Serial: {:?}",
        passed + 1,
        if passed < 6 { fib[passed].1 } else { 0 },
        format_registers(cpu),
        serial_output,
    );
}

/// Drive a Blargg ROM until it reports over the serial link, and require a pass.
pub fn assert_blargg_serial<S: System>(s: &mut S, rom_path: &str, timeout_frames: u32) {
    let output = run_until_serial_match(s, &["Passed", "Failed"], timeout_frames);
    assert!(
        output.contains("Passed"),
        "Blargg test {rom_path} failed. Serial output:\n{output}"
    );
}

/// Drive a Blargg ROM to its completion loop and compare its result screen
/// against a greyscale reference.
pub fn assert_blargg_screen<S: System>(
    s: &mut S,
    rom_path: &str,
    reference: &str,
    timeout_frames: u32,
) {
    let found_loop = run_until_infinite_loop(s, timeout_frames);
    assert!(
        found_loop,
        "Blargg test {rom_path} timed out without reaching infinite loop"
    );
    assert_screen_matches(
        &format!("Blargg test {rom_path} vs {reference}"),
        &s.screen_greyscale(),
        reference,
    );
}

/// Drive a Scribbltest to its `LD B,B` breakpoint and compare its screen.
pub fn assert_scribbltest<S: System>(s: &mut S, rom_name: &str, timeout_frames: u32) {
    let found_breakpoint = run_until_breakpoint(s, timeout_frames);
    assert!(
        found_breakpoint,
        "Scribbltest {rom_name} timed out without reaching LD B,B breakpoint"
    );
    assert_screen_matches(
        &format!("Scribbltest {rom_name}"),
        &s.screen_greyscale(),
        &format!("scribbltests/{rom_name}-dmg.png"),
    );
}

/// Run a TurtleTest long enough to display its result and compare the screen.
/// The ROMs don't terminate in a loop, so this is a fixed frame budget.
pub fn assert_turtle_test<S: System>(s: &mut S, rom_name: &str) {
    run_frames(s, 30);
    assert_screen_matches(
        &format!("TurtleTest {rom_name}"),
        &s.screen_greyscale(),
        &format!("turtle-tests/{rom_name}-dmg.png"),
    );
}

// Decodes the wilbertpol mooneye fork's `Runtime-State` ramsection — `regs_save`
// (actual values), `regs_flags` (bit-per-assertion), `regs_assert` (expected values).
// Layout is: 8 + 1 + 8 = 17 bytes at WRAM slot 2 base. Base is wlalink's convention
// for unpositioned ramsections, not a source-pinned address.
const RECORD_BASE: u16 = 0xC000;
const RECORD_LEN: u16 = 17;

#[derive(Clone, Copy)]
enum AssertReg {
    A,
    F,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl AssertReg {
    const ITER: [AssertReg; 8] = [
        AssertReg::A,
        AssertReg::F,
        AssertReg::B,
        AssertReg::C,
        AssertReg::D,
        AssertReg::E,
        AssertReg::H,
        AssertReg::L,
    ];

    fn flag_bit(self) -> u8 {
        match self {
            AssertReg::A => 0,
            AssertReg::F => 1,
            AssertReg::B => 2,
            AssertReg::C => 3,
            AssertReg::D => 4,
            AssertReg::E => 5,
            AssertReg::H => 6,
            AssertReg::L => 7,
        }
    }

    // Byte offset within `reg_dump`. Order is `f, a, c, b, e, d, l, h`, matching
    // `push hl; push de; push bc; push af` from `regs_save + 8` (low to high).
    fn dump_offset(self) -> usize {
        match self {
            AssertReg::F => 0,
            AssertReg::A => 1,
            AssertReg::C => 2,
            AssertReg::B => 3,
            AssertReg::E => 4,
            AssertReg::D => 5,
            AssertReg::L => 6,
            AssertReg::H => 7,
        }
    }

    fn label(self) -> &'static str {
        match self {
            AssertReg::A => "a",
            AssertReg::F => "f",
            AssertReg::B => "b",
            AssertReg::C => "c",
            AssertReg::D => "d",
            AssertReg::E => "e",
            AssertReg::H => "h",
            AssertReg::L => "l",
        }
    }
}

struct FailedAssertion {
    reg: AssertReg,
    expected: u8,
    actual: u8,
}

fn decode_assertion_record<S: System>(s: &S) -> (u8, Vec<FailedAssertion>) {
    let bytes = s.peek_range(RECORD_BASE, RECORD_LEN);
    let save = &bytes[0..8];
    let flags = bytes[8];
    let assert = &bytes[9..17];

    let mut failed = Vec::new();
    for reg in AssertReg::ITER {
        if flags & (1 << reg.flag_bit()) == 0 {
            continue;
        }
        let off = reg.dump_offset();
        if save[off] != assert[off] {
            failed.push(FailedAssertion {
                reg,
                expected: assert[off],
                actual: save[off],
            });
        }
    }
    (flags, failed)
}

/// Drive a wilbertpol-fork Mooneye ROM to its `0xED` exit and require the
/// Fibonacci pass registers, decoding the WRAM assertion record on failure.
pub fn assert_wilbertpol_verdict<S: System>(s: &mut S, rom_path: &str, timeout_frames: u32) {
    let found = run_until_undefined_opcode(s, timeout_frames);
    assert!(
        found,
        "Mooneye-wilbertpol test {rom_path} timed out without reaching exit condition"
    );
    let cpu = s.cpu();
    if check_mooneye_pass(cpu) {
        return;
    }

    let (flags, failed) = decode_assertion_record(s);
    if !failed.is_empty() {
        let mut msg = format!(
            "Mooneye-wilbertpol test {rom_path} failed: {} assertion(s)",
            failed.len()
        );
        for f in &failed {
            msg.push_str(&format!(
                "\n  assert_{}: expected 0x{:02X}, got 0x{:02X}",
                f.reg.label(),
                f.expected,
                f.actual,
            ));
        }
        panic!("{msg}");
    }

    // The harness stores the failing testcase_id at the record base.
    let testcase_id = s.peek_range(RECORD_BASE, 1)[0];
    let round = match cpu.c {
        0x49 => "Round A FAILED (mode-3 too LONG)",
        0xBA => "Round B FAILED (mode-3 too SHORT)",
        0x17 => "STAT IRQ never fired",
        _ => "(unknown failure round)",
    };
    panic!(
        "Mooneye-wilbertpol test {rom_path} failed with no per-assertion mismatch \
         (regs_flags=0x{flags:02X}). testcase_id=0x{testcase_id:02X}, {round}. \
         Registers: {}",
        format_registers(cpu),
    );
}

/// The 8x8 glyphs the gambatte suite prints its expected value in, one per hex
/// digit: 0 is foreground, 1 background.
#[rustfmt::skip]
const HEX_TILES: [[u8; 64]; 16] = [
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,0], // 0
    [1,1,1,1,1,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1, 1,1,1,1,0,1,1,1], // 1
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,0], // 2
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,1,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,0,0,0,0,0,0,0], // 3
    [1,1,1,1,1,1,1,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0], // 4
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,1, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,0,0,0,0,0,0,1], // 5
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,0], // 6
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,0,1, 1,1,1,1,1,0,1,1, 1,1,1,1,0,1,1,1, 1,1,1,0,1,1,1,1, 1,1,1,0,1,1,1,1], // 7
    [1,1,1,1,1,1,1,1, 1,1,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,1,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,1,0,0,0,0,0,1], // 8
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,0, 1,1,1,1,1,1,1,0, 1,1,1,1,1,1,1,0, 1,0,0,0,0,0,0,0], // 9
    [1,1,1,1,1,1,1,1, 1,1,1,1,0,1,1,1, 1,1,0,1,1,1,0,1, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0], // A
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,1], // B
    [1,1,1,1,1,1,1,1, 1,1,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,0, 1,1,0,0,0,0,0,1], // C
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,1, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,1,1,1,1,1,0, 1,0,0,0,0,0,0,1], // D
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,0], // E
    [1,1,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,0,0,0,0,0,0, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1, 1,0,1,1,1,1,1,1], // F
];

/// Does the screen's top-left row of tiles spell `expected_hex`? Each digit
/// occupies an 8x8 tile at (digit index × 8, 0), matched to within the ±8
/// tolerance gambatte's own 0xF8F8F8 mask allows.
pub fn screen_matches_hex(screen_greyscale: &[u8], expected_hex: &str) -> bool {
    let digits: Vec<u8> = expected_hex
        .chars()
        .map(|c| {
            c.to_digit(16)
                .unwrap_or_else(|| panic!("Invalid hex char: {c}")) as u8
        })
        .collect();
    for (idx, &digit) in digits.iter().enumerate() {
        let x_off = idx * 8;
        if x_off + 8 > 160 {
            break;
        }
        if !hex_tile_matches(screen_greyscale, x_off, digit) {
            return false;
        }
    }
    true
}

/// Does the 8×8 tile at `x_off` on the screen's top row show `digit`?
fn hex_tile_matches(screen_greyscale: &[u8], x_off: usize, digit: u8) -> bool {
    let tile = &HEX_TILES[digit as usize];
    (0..8).all(|ty| {
        (0..8).all(|tx| {
            let screen_pixel = screen_greyscale[ty * 160 + x_off + tx];
            let expected_pixel = if tile[ty * 8 + tx] == 0 { 0x00 } else { 0xFF };
            (screen_pixel as i16 - expected_pixel as i16).unsigned_abs() <= 8
        })
    })
}

/// Reverse of [`screen_matches_hex`]: read back the hex digits the screen
/// actually shows, for diagnostics on failure. A digit slot that matches no
/// tile (e.g. a blank screen) reads as `?`.
pub fn decode_screen_hex(screen_greyscale: &[u8], num_digits: usize) -> String {
    (0..num_digits)
        .map(|idx| {
            let x_off = idx * 8;
            if x_off + 8 > 160 {
                return '?';
            }
            for digit in 0..HEX_TILES.len() as u8 {
                if hex_tile_matches(screen_greyscale, x_off, digit) {
                    return char::from_digit(digit as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase();
                }
            }
            '?'
        })
        .collect()
}
