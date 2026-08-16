//! SingleStepTests 65x02 oracle sweep: per-cycle bus activity and final
//! state for all 256 opcodes. Data is fetched (not committed) — the sweep
//! sparse-clones the oracle on first run, or honours `SINGLE_STEP_TESTS_DIR`.

use std::path::PathBuf;

use missingno_mos_6502::{Bus, Cpu};
use missingno_test_support::oracle::fetch_oracle;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    initial: CpuState,
    #[serde(rename = "final")]
    end: CpuState,
    cycles: Vec<(u16, u8, String)>,
}

#[derive(Deserialize)]
struct CpuState {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    ram: Vec<(u16, u8)>,
}

struct FlatBus {
    memory: Vec<u8>,
    log: Vec<(u16, u8, bool)>,
}

impl FlatBus {
    fn new() -> Self {
        FlatBus {
            memory: vec![0; 0x10000],
            log: Vec::new(),
        }
    }
}

impl Bus for FlatBus {
    fn read(&mut self, address: u16) -> u8 {
        let value = self.memory[address as usize];
        self.log.push((address, value, false));
        value
    }

    fn write(&mut self, address: u16, data: u8) {
        self.memory[address as usize] = data;
        self.log.push((address, data, true));
    }
}

fn data_dir(variant: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("SINGLE_STEP_TESTS_DIR") {
        return PathBuf::from(dir).join(variant).join("v1");
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/single-step-tests");
    let dir = root.join(variant).join("v1");
    if !dir.is_dir() {
        fetch_oracle(
            &root,
            "https://github.com/SingleStepTests/65x02",
            &["6502/v1", "nes6502/v1"],
            "SINGLE_STEP_TESTS_DIR",
        );
    }
    dir
}

fn run_case(case: &Case, decimal: bool) -> Result<(), String> {
    let mut cpu = if decimal {
        Cpu::new()
    } else {
        Cpu::new_without_decimal()
    };
    cpu.pc = case.initial.pc;
    cpu.s = case.initial.s;
    cpu.a = case.initial.a;
    cpu.x = case.initial.x;
    cpu.y = case.initial.y;
    cpu.p = case.initial.p;

    let mut bus = FlatBus::new();
    for &(address, value) in &case.initial.ram {
        bus.memory[address as usize] = value;
    }

    for _ in 0..case.cycles.len() {
        cpu.tick(&mut bus);
    }
    if !cpu.at_instruction_boundary() && !cpu.jammed() {
        return Err(format!("did not finish in {} cycles", case.cycles.len()));
    }

    let mut problems = Vec::new();
    let end = &case.end;
    for (label, got, want) in [
        ("pc", cpu.pc as u32, end.pc as u32),
        ("s", cpu.s as u32, end.s as u32),
        ("a", cpu.a as u32, end.a as u32),
        ("x", cpu.x as u32, end.x as u32),
        ("y", cpu.y as u32, end.y as u32),
        ("p", cpu.p as u32, end.p as u32),
    ] {
        if got != want {
            problems.push(format!("{label}: got {got:02X} want {want:02X}"));
        }
    }
    for &(address, want) in &end.ram {
        let got = bus.memory[address as usize];
        if got != want {
            problems.push(format!("ram[{address:04X}]: got {got:02X} want {want:02X}"));
        }
    }
    let want_cycles: Vec<(u16, u8, bool)> = case
        .cycles
        .iter()
        .map(|(address, value, kind)| (*address, *value, kind == "write"))
        .collect();
    if bus.log != want_cycles {
        problems.push(format!(
            "cycles: got {:X?} want {:X?}",
            bus.log, want_cycles
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn run_file(path: &PathBuf, decimal: bool) -> (usize, usize, Vec<String>) {
    let raw = std::fs::read(path).expect("readable test file");
    let cases: Vec<Case> = serde_json::from_slice(&raw).expect("valid test JSON");
    let mut passed = 0;
    let mut failed = 0;
    let mut examples = Vec::new();
    for case in &cases {
        match run_case(case, decimal) {
            Ok(()) => passed += 1,
            Err(problem) => {
                failed += 1;
                if examples.len() < 3 {
                    examples.push(format!("  {}: {problem}", case.name));
                }
            }
        }
    }
    (passed, failed, examples)
}

fn sweep(variant: &str, decimal: bool) {
    let dir = data_dir(variant);
    let mut total_passed = 0usize;
    let mut bad_files = Vec::new();
    for opcode in 0..=0xFFu8 {
        let path = dir.join(format!("{opcode:02x}.json"));
        if !path.exists() {
            continue;
        }
        let (passed, failed, examples) = run_file(&path, decimal);
        total_passed += passed;
        if failed > 0 {
            bad_files.push(format!(
                "{opcode:02X}: {failed} failed\n{}",
                examples.join("\n")
            ));
        }
    }
    assert!(
        bad_files.is_empty(),
        "{} opcode files with failures (passed {total_passed}):\n{}",
        bad_files.len(),
        bad_files.join("\n")
    );
    assert!(
        total_passed > 2_000_000,
        "suspiciously few cases ran: {total_passed}"
    );
}

#[test]
fn single_step_sweep() {
    sweep("6502", true);
}

#[test]
fn single_step_sweep_nes6502() {
    sweep("nes6502", false);
}

/// Data-free sanity check so plain `cargo test` exercises the core.
#[test]
fn lda_immediate_smoke() {
    let mut cpu = Cpu::new();
    let mut bus = FlatBus::new();
    bus.memory[0x0200] = 0xA9; // LDA #$80
    bus.memory[0x0201] = 0x80;
    cpu.pc = 0x0200;
    cpu.tick(&mut bus);
    cpu.tick(&mut bus);
    assert!(cpu.at_instruction_boundary());
    assert_eq!(cpu.a, 0x80);
    assert_eq!(cpu.p & 0x80, 0x80);
    assert_eq!(cpu.pc, 0x0202);
}
